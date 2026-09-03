use std::collections::{BTreeMap, BTreeSet};

use crate::asm6502::{InlineAsmMode, InlineAsmRelocationKind, InlineAsmSymbolUse};
use crate::ast::{
    AddressByteSelector, BinaryOp, FundType, MachineAddressAtom, MachineItem, UnaryOp,
};
use crate::lexer::{TokenKind, tokenize};
use crate::resident::{ResidentVariableKind, resident_variable};
use crate::semantic::{
    ArrayType, SymbolClass, SymbolId as SemSymbolId, ValueType, ValueTypeKind,
    ir::{
        SemArrayOrigin, SemCall, SemCallable, SemCondition, SemConditionKind, SemDeclaration,
        SemDeclarationStorage, SemEffects, SemExpr, SemExprClass, SemExprKind, SemForStep,
        SemInitializerElement, SemInitializerElementKind, SemInitializerLiteral, SemInlineAsm,
        SemInlineAsmTarget, SemLValue, SemLValueKind, SemLiteral, SemMachineSymbolRef, SemProgram,
        SemReadEffect, SemSet, SemStaticInitializer, SemStaticInitializerValue, SemStmt,
        SemStorageRef, SemSymbolRef, SemWriteEffect,
    },
};
use crate::source::source_char_byte;
use crate::target::{AddressValue, ByteOffset, ByteSize, TargetLayout};

use super::classifier::NirClassifier;
use super::facts::{
    BlockId, LocalId, NirFacts, NirStorageId, NirType, NirTypeKind, NirValue, ParamId, SymbolId,
    TempId, signature_id, type_summary,
};
use super::ir::*;

#[derive(Default)]
pub(super) struct NirLowerer {
    target_layout: TargetLayout,
    next_label: usize,
    next_global: u32,
    next_static: u32,
    global_ids: BTreeMap<String, SymbolId>,
    global_ids_by_symbol: BTreeMap<SemSymbolId, SymbolId>,
    routine_ids: BTreeMap<String, u32>,
    routine_types: BTreeMap<String, NirType>,
    symbol_storage_types: BTreeMap<String, NirType>,
    semantic_storage_types: BTreeMap<SemSymbolId, NirType>,
    semantic_absolute_globals: BTreeMap<SemSymbolId, u16>,
    semantic_absolute_array_element_bases: BTreeMap<SemSymbolId, u16>,
    semantic_absolute_array_value_addresses: BTreeMap<SemSymbolId, u16>,
    storage_symbol_ids: BTreeSet<SemSymbolId>,
    compatible_cursor: Option<u16>,
    machine_defines: BTreeMap<usize, Vec<MachineItem>>,
    machine_define_names: BTreeMap<String, Vec<MachineItem>>,
}

impl NirLowerer {
    pub(super) fn program(&mut self, program: &SemProgram) -> NirProgram {
        self.target_layout = program.target_layout;
        let mut globals = Vec::new();
        let mut statics = Vec::new();
        let mut routines = Vec::new();
        let mut top_level_ops = Vec::new();
        let mut top_level = Vec::new();

        self.collect_global_ids(program);
        self.collect_routine_ids(program);
        let machine_defines = collect_machine_defines(program);
        self.machine_define_names = machine_defines.names;
        self.machine_defines = machine_defines.ids;
        let record_storage_sizes = record_storage_sizes(program);
        let program_entry = program
            .program_entry_routine()
            .map(|routine| routine.symbol.id);

        for module in &program.modules {
            for item in &module.items {
                match item {
                    crate::semantic::ir::SemItem::Define(define) => {
                        let id = self.global_id(&define.symbol.name);
                        globals.push(NirGlobal {
                            id,
                            name: define.symbol.name.clone(),
                            kind: format!("define {}", define.value),
                            ty: None,
                            storage_size: ByteSize::ZERO,
                            array: None,
                            init: None,
                            backing: NirGlobalBacking::Ordinary,
                        });
                    }
                    crate::semantic::ir::SemItem::Const(_) => {}
                    crate::semantic::ir::SemItem::Include(include) => {
                        let id = self.global_id(&include.path);
                        globals.push(NirGlobal {
                            id,
                            name: include.path.clone(),
                            kind: "include".to_string(),
                            ty: None,
                            storage_size: ByteSize::ZERO,
                            array: None,
                            init: None,
                            backing: NirGlobalBacking::Ordinary,
                        });
                    }
                    crate::semantic::ir::SemItem::Set(set) => {
                        if apply_program_end_symbol_set(&mut globals, set) {
                            continue;
                        }
                        self.apply_compatible_set(set);
                        if let Some(op) = self.runtime_helper_override(set) {
                            top_level_ops.push(op);
                        }
                    }
                    crate::semantic::ir::SemItem::Declaration(declaration) => {
                        let id = self.global_id(&declaration.symbol.name);
                        let address_initializer = match &declaration.storage {
                            SemDeclarationStorage::Array {
                                fixed_address: Some(address),
                                ..
                            } => Some(*address),
                            _ => declaration
                                .initializer
                                .as_ref()
                                .and_then(|expr| self.const_u16_expr(expr)),
                        };
                        if let Some(ty) =
                            declaration_symbol_storage_type(declaration, address_initializer)
                        {
                            self.symbol_storage_types
                                .insert(declaration.symbol.name.clone(), ty.clone());
                            self.semantic_storage_types
                                .insert(declaration.symbol.id, ty);
                        }
                        let alias_initializer = self.scalar_storage_alias_initializer(declaration);
                        let backing = self.declaration_backing(
                            declaration,
                            &record_storage_sizes,
                            address_initializer,
                            alias_initializer,
                        );
                        if let NirGlobalBacking::Absolute(address) = backing {
                            self.semantic_absolute_globals
                                .insert(declaration.symbol.id, address.value as u16);
                            if declaration_is_array(declaration) {
                                self.semantic_absolute_array_element_bases
                                    .insert(declaration.symbol.id, address.value as u16);
                            }
                        }
                        if let Some(address) = address_initializer
                            && declaration_array_address_initializer_uses_pointer_storage(
                                declaration,
                                &record_storage_sizes,
                                self.target_layout,
                            )
                        {
                            self.semantic_absolute_array_value_addresses
                                .insert(declaration.symbol.id, address);
                        }
                        globals.push(NirGlobal {
                            id,
                            name: declaration.symbol.name.clone(),
                            kind: declaration_kind(declaration),
                            ty: Some(NirFacts::type_from_value(&declaration.ty.value)),
                            storage_size: ByteSize::from(declaration_storage_size(
                                declaration,
                                &record_storage_sizes,
                                address_initializer,
                                self.target_layout,
                            )),
                            array: declaration_array_fact(
                                declaration,
                                &record_storage_sizes,
                                address_initializer,
                                self.target_layout,
                            ),
                            init: declaration_global_init(
                                id,
                                declaration,
                                &record_storage_sizes,
                                &backing,
                                address_initializer,
                                &self.global_ids,
                                &self.routine_ids,
                                self.target_layout,
                            ),
                            backing,
                        });
                        self.storage_symbol_ids.insert(declaration.symbol.id);
                    }
                    crate::semantic::ir::SemItem::Routine(routine) => {
                        let routine_name = if routine.is_external {
                            &routine.symbol.qualified_name
                        } else {
                            &routine.symbol.name
                        };
                        let mut builder = NirBuilder::new(
                            routine_name,
                            self.next_block_label(),
                            self.next_static,
                            self.global_ids.clone(),
                            self.global_ids_by_symbol.clone(),
                            self.routine_ids.clone(),
                            self.routine_types.clone(),
                            self.symbol_storage_types.clone(),
                            self.semantic_storage_types.clone(),
                            self.semantic_absolute_array_element_bases.clone(),
                            self.semantic_absolute_array_value_addresses.clone(),
                            record_storage_sizes.clone(),
                            self.machine_defines.clone(),
                            self.machine_define_names.clone(),
                            self.target_layout,
                        );
                        for (index, param) in routine.params.iter().enumerate() {
                            let ty = match param.storage {
                                crate::semantic::ir::SemParamStorage::Value => {
                                    param.ty.value.clone()
                                }
                                crate::semantic::ir::SemParamStorage::Array => {
                                    crate::semantic::ValueType::pointer_to(param.ty.value.clone())
                                }
                            };
                            let ty = NirFacts::type_from_value(&ty);
                            if matches!(param.storage, crate::semantic::ir::SemParamStorage::Array)
                            {
                                builder
                                    .symbol_storage_types
                                    .insert(param.symbol.name.clone(), ty.clone());
                                builder
                                    .semantic_storage_types
                                    .insert(param.symbol.id, ty.clone());
                            }
                            builder
                                .param_ids_by_symbol
                                .insert(param.symbol.id, ParamId(index as u32));
                            builder.params.push(NirParam {
                                id: ParamId(index as u32),
                                name: param.symbol.name.clone(),
                                storage: match param.storage {
                                    crate::semantic::ir::SemParamStorage::Value => {
                                        NirStorageClass::Scalar
                                    }
                                    crate::semantic::ir::SemParamStorage::Array => {
                                        NirStorageClass::Array
                                    }
                                },
                                ty,
                            });
                        }
                        let mut all_declarations = routine.locals.iter().collect::<Vec<_>>();
                        collect_nested_declarations(&routine.body, &mut all_declarations);
                        let all_locals = all_declarations
                            .into_iter()
                            .filter(|declaration| declaration_has_storage(declaration))
                            .collect::<Vec<_>>();
                        let mut local_alias_targets = BTreeMap::new();
                        let param_ids_by_symbol = routine
                            .params
                            .iter()
                            .enumerate()
                            .map(|(index, param)| (param.symbol.id, ParamId(index as u32)))
                            .collect::<BTreeMap<_, _>>();
                        let local_ids_by_symbol = all_locals
                            .iter()
                            .enumerate()
                            .map(|(index, local)| (local.symbol.id, LocalId(index as u32)))
                            .collect::<BTreeMap<_, _>>();
                        builder.param_ids_by_symbol = param_ids_by_symbol.clone();
                        builder.local_ids_by_symbol = local_ids_by_symbol.clone();
                        for (index, local) in all_locals.into_iter().enumerate() {
                            let address_initializer = match &local.storage {
                                SemDeclarationStorage::Array {
                                    fixed_address: Some(address),
                                    ..
                                } => Some(*address),
                                _ => local
                                    .initializer
                                    .as_ref()
                                    .and_then(|expr| self.const_u16_expr(expr)),
                            };
                            let backing = self.local_backing(
                                local,
                                &record_storage_sizes,
                                address_initializer,
                                &local_alias_targets,
                            );
                            if let NirLocalBacking::Absolute(address) = backing
                                && declaration_is_array(local)
                            {
                                builder
                                    .semantic_absolute_array_element_bases
                                    .insert(local.symbol.id, address.value as u16);
                            }
                            if let Some(ty) =
                                declaration_symbol_storage_type(local, address_initializer)
                            {
                                builder
                                    .symbol_storage_types
                                    .insert(local.symbol.name.clone(), ty.clone());
                                builder.semantic_storage_types.insert(local.symbol.id, ty);
                            }
                            builder.locals.push(NirLocal {
                                id: LocalId(index as u32),
                                name: semantic_local_display_name(&local.symbol),
                                kind: declaration_kind(local),
                                purpose: NirLocalPurpose::Storage,
                                storage: declaration_storage_class(&local.storage),
                                ty: NirFacts::type_from_value(&local.ty.value),
                                init: declaration_local_init(
                                    local,
                                    &record_storage_sizes,
                                    &backing,
                                    &self.global_ids,
                                    &self.routine_ids,
                                    &param_ids_by_symbol,
                                    &local_ids_by_symbol,
                                    self.target_layout,
                                ),
                                backing,
                            });
                            local_alias_targets.insert(
                                local.symbol.id,
                                (
                                    LocalId(index as u32),
                                    semantic_local_display_name(&local.symbol),
                                ),
                            );
                        }
                        if let Some(return_type) = routine.callable_type.return_type.as_ref() {
                            let return_type = NirFacts::type_from_value(return_type);
                            if let Some(width) = return_type.width {
                                builder.notes.push(NirRoutineNote {
                                    text: format!("return-width {width}"),
                                    kind: NirRoutineNoteKind::Informational,
                                });
                            }
                        }
                        if routine.is_external {
                            builder.notes.push(NirRoutineNote {
                                text: routine.symbol.canonical_qualified_key.clone(),
                                kind: NirRoutineNoteKind::ExternalInterface,
                            });
                        }
                        if program_entry == Some(routine.symbol.id) {
                            builder.notes.push(NirRoutineNote {
                                text: "Action program entry (last source PROC)".to_string(),
                                kind: NirRoutineNoteKind::ProgramEntry,
                            });
                        }
                        if let Some(address) = &routine.system_address {
                            builder.notes.push(NirRoutineNote {
                                text: format!("system-address {}", expr_summary(address)),
                                kind: if matches!(address.kind, SemExprKind::CurrentLocation) {
                                    NirRoutineNoteKind::CurrentLocationEntry
                                } else {
                                    NirRoutineNoteKind::Informational
                                },
                            });
                        }
                        for (name, items) in machine_define_names_from_statements(&routine.body) {
                            builder.machine_define_names.insert(name, items);
                        }
                        builder.stmt_list(&routine.body, self);
                        builder.finish_open_with(NirTerminator::Fallthrough);
                        let (routine, routine_statics, next_static) = builder.finish();
                        self.next_static = next_static;
                        statics.extend(routine_statics);
                        routines.push(routine);
                    }
                    crate::semantic::ir::SemItem::Statement(stmt) => top_level.push(stmt.clone()),
                    crate::semantic::ir::SemItem::Unsupported { span, note } => {
                        top_level.push(SemStmt::Unsupported {
                            span: *span,
                            note: note.clone(),
                        });
                    }
                }
            }
        }

        if !top_level_ops.is_empty() || !top_level.is_empty() {
            let mut builder = NirBuilder::new(
                "<program>",
                self.next_block_label(),
                self.next_static,
                self.global_ids.clone(),
                self.global_ids_by_symbol.clone(),
                self.routine_ids.clone(),
                self.routine_types.clone(),
                self.symbol_storage_types.clone(),
                self.semantic_storage_types.clone(),
                self.semantic_absolute_array_element_bases.clone(),
                self.semantic_absolute_array_value_addresses.clone(),
                record_storage_sizes.clone(),
                self.machine_defines.clone(),
                self.machine_define_names.clone(),
                self.target_layout,
            );
            for op in top_level_ops {
                builder.push(op);
            }
            builder.stmt_list(&top_level, self);
            builder.finish_open_with(NirTerminator::Fallthrough);
            let (routine, routine_statics, next_static) = builder.finish();
            self.next_static = next_static;
            statics.extend(routine_statics);
            routines.insert(0, routine);
            for routine in &mut routines {
                for local in &mut routine.locals {
                    if let Some(init) = &mut local.init {
                        increment_data_image_routine_ids_in_storage_init(init);
                    }
                }
                for block in &mut routine.blocks {
                    for op in &mut block.ops {
                        match op {
                            NirOp::RuntimeHelperOverride {
                                target: NirRuntimeHelperTarget::Routine(id),
                                ..
                            } => *id = id.saturating_add(1),
                            NirOp::Call {
                                callee: NirCallee::User { id, .. },
                                ..
                            } => *id = id.saturating_add(1),
                            NirOp::MachineBlock { items, .. } => {
                                for item in items {
                                    if let NirMachineItem::Relocation {
                                        target: NirInlineAsmTarget::Routine(id),
                                        ..
                                    } = item
                                    {
                                        *id = id.saturating_add(1);
                                    }
                                }
                            }
                            NirOp::InlineAsm { code, .. } => {
                                for relocation in &mut code.relocations {
                                    if let NirInlineAsmTarget::Routine(id) = &mut relocation.target
                                    {
                                        *id = id.saturating_add(1);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            for global in &mut globals {
                if let Some(init) = &mut global.init {
                    increment_data_image_routine_ids_in_global_init(init);
                }
            }
            for static_data in &mut statics {
                increment_data_image_routine_ids(&mut static_data.image);
            }
        }

        deduplicate_real_statics(&mut statics, &mut routines);

        let mut nir = NirProgram {
            target_layout: program.target_layout,
            globals,
            statics,
            routines,
        };
        apply_target_layout_to_program(&mut nir);
        nir
    }

    fn next_block_label(&mut self) -> String {
        let label = format!("bb{}", self.next_label);
        self.next_label += 1;
        label
    }

    fn next_global_id(&mut self) -> SymbolId {
        let id = SymbolId(self.next_global);
        self.next_global += 1;
        id
    }

    fn global_id(&self, name: &str) -> SymbolId {
        *self
            .global_ids
            .get(name)
            .expect("global id collection should predeclare all global symbols")
    }

    fn collect_global_ids(&mut self, program: &SemProgram) {
        for module in &program.modules {
            for item in &module.items {
                let (name, semantic_symbol) = match item {
                    crate::semantic::ir::SemItem::Define(define) => {
                        (Some(&define.symbol.name), Some(define.symbol.id))
                    }
                    crate::semantic::ir::SemItem::Const(_) => (None, None),
                    crate::semantic::ir::SemItem::Include(include) => (Some(&include.path), None),
                    crate::semantic::ir::SemItem::Declaration(declaration) => {
                        (Some(&declaration.symbol.name), Some(declaration.symbol.id))
                    }
                    crate::semantic::ir::SemItem::Routine(routine) => {
                        (Some(&routine.symbol.name), Some(routine.symbol.id))
                    }
                    crate::semantic::ir::SemItem::Set(_)
                    | crate::semantic::ir::SemItem::Statement(_)
                    | crate::semantic::ir::SemItem::Unsupported { .. } => (None, None),
                };
                if let Some(name) = name {
                    let id = if let Some(id) = self.global_ids.get(name).copied() {
                        id
                    } else {
                        let id = self.next_global_id();
                        self.global_ids.insert(name.clone(), id);
                        id
                    };
                    if let Some(symbol) = semantic_symbol {
                        self.global_ids_by_symbol.insert(symbol, id);
                    }
                }
            }
        }
    }

    fn collect_routine_ids(&mut self, program: &SemProgram) {
        self.routine_ids.clear();
        self.routine_types.clear();
        for module in &program.modules {
            for item in &module.items {
                if let crate::semantic::ir::SemItem::Routine(routine) = item {
                    let id = self.routine_ids.len() as u32;
                    self.routine_ids
                        .insert(storage_key(&routine.symbol.name), id);
                    self.routine_types.insert(
                        storage_key(&routine.symbol.name),
                        NirType::from_value(&ValueType::callable_pointer(
                            routine.callable_type.clone(),
                        )),
                    );
                }
            }
        }
    }

    fn declaration_backing(
        &mut self,
        declaration: &SemDeclaration,
        record_storage_sizes: &BTreeMap<String, u16>,
        address_initializer: Option<u16>,
        alias_initializer: Option<(SemSymbolId, String, u16)>,
    ) -> NirGlobalBacking {
        if let Some(address) = address_initializer {
            if declaration_array_address_initializer_uses_pointer_storage(
                declaration,
                record_storage_sizes,
                self.target_layout,
            ) {
                return NirGlobalBacking::Ordinary;
            }
            return NirGlobalBacking::Absolute(AddressValue::data(u64::from(address)));
        }
        if let Some((target_symbol, _target_name, offset)) = alias_initializer {
            if let Some(target) = self.global_ids_by_symbol.get(&target_symbol).copied() {
                return NirGlobalBacking::Alias {
                    target,
                    offset: ByteOffset::from(offset),
                };
            }
        }

        let Some(address) = self.compatible_cursor else {
            return NirGlobalBacking::Ordinary;
        };
        let size = declaration_storage_size(
            declaration,
            record_storage_sizes,
            address_initializer,
            self.target_layout,
        );
        self.compatible_cursor = Some(address.wrapping_add(size));
        NirGlobalBacking::Absolute(AddressValue::data(u64::from(address)))
    }

    fn scalar_storage_alias_initializer(
        &self,
        declaration: &SemDeclaration,
    ) -> Option<(SemSymbolId, String, u16)> {
        if !matches!(declaration.storage, SemDeclarationStorage::Scalar)
            || declaration.ty.value.pointer
        {
            return None;
        }
        let initializer = declaration.initializer.as_ref()?;
        let (target, offset) = storage_alias_initializer_expr(initializer)?;
        if !self.storage_symbol_ids.contains(&target.id) {
            return None;
        }
        Some((target.id, target.name.clone(), offset))
    }

    fn apply_compatible_set(&mut self, set: &SemSet) {
        if self.apply_compatible_symbol_set(set) {
            return;
        }
        let Some(address) = self.const_u16_expr(&set.address) else {
            return;
        };
        let Some(value) = self.const_u16_expr(&set.value) else {
            return;
        };
        match address {
            0x000E | 0x0491 => self.compatible_cursor = (value < 0x0100).then_some(value),
            0x000F | 0x0492 => {
                let current = self.compatible_cursor.unwrap_or(0);
                let updated = (current & 0x00FF) | ((value & 0x00FF) << 8);
                self.compatible_cursor = (updated < 0x0100).then_some(updated);
            }
            _ => {}
        }
    }

    fn runtime_helper_override(&self, set: &SemSet) -> Option<NirOp> {
        let slot = self.const_u16_expr(&set.address)?;
        if !is_runtime_helper_slot(slot) {
            return None;
        }
        let target = match &set.value.kind {
            SemExprKind::Symbol(symbol) | SemExprKind::AddressOfSymbol(symbol)
                if matches!(
                    symbol.class,
                    SymbolClass::Proc
                        | SymbolClass::Func
                        | SymbolClass::BuiltinProc
                        | SymbolClass::BuiltinFunc
                ) =>
            {
                NirRuntimeHelperTarget::Routine(
                    *self
                        .routine_ids
                        .get(&storage_key(&symbol.name))
                        .expect("resolved helper override must have a routine id"),
                )
            }
            _ => NirRuntimeHelperTarget::Absolute(AddressValue::code(u64::from(
                self.const_u16_expr(&set.value)?,
            ))),
        };
        Some(NirOp::RuntimeHelperOverride {
            slot: AddressValue::data(u64::from(slot)),
            target,
        })
    }

    fn apply_compatible_symbol_set(&mut self, set: &SemSet) -> bool {
        let SemExprKind::LValue(lvalue) = &set.address.kind else {
            return false;
        };
        let SemLValueKind::Symbol(symbol) = &lvalue.kind else {
            return false;
        };
        let Some(value) = self.const_u16_expr(&set.value) else {
            return false;
        };
        self.semantic_absolute_globals.insert(symbol.id, value);
        true
    }

    fn const_u16_expr(&self, expr: &SemExpr) -> Option<u16> {
        let storage_address = matches!(&expr.kind, SemExprKind::Symbol(_) | SemExprKind::LValue(_));
        let value = match &expr.kind {
            SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
            SemExprKind::Literal(SemLiteral::Constant(value)) => Some(value.bits),
            SemExprKind::Symbol(symbol) => self.semantic_absolute_globals.get(&symbol.id).copied(),
            SemExprKind::LValue(lvalue) => self.const_u16_lvalue(lvalue),
            SemExprKind::Cast { expr, .. } => self.const_u16_expr(expr),
            SemExprKind::Unary { op, expr } => {
                let value = self.const_u16_expr(expr)?;
                match op {
                    UnaryOp::Plus => Some(value),
                    UnaryOp::Neg => Some(0u16.wrapping_sub(value)),
                    UnaryOp::AddressOf | UnaryOp::Deref => None,
                }
            }
            SemExprKind::Binary { op, left, right } => {
                let left = self.const_u16_expr(left)?;
                let right = self.const_u16_expr(right)?;
                match op {
                    BinaryOp::Add => Some(left.wrapping_add(right)),
                    BinaryOp::Sub => Some(left.wrapping_sub(right)),
                    BinaryOp::Mul => Some(left.wrapping_mul(right)),
                    BinaryOp::Div => (right != 0).then_some(left / right),
                    BinaryOp::Mod => (right != 0).then_some(left % right),
                    BinaryOp::Lsh => Some(left.wrapping_shl(u32::from(right & 0x0F))),
                    BinaryOp::Rsh => Some(left.wrapping_shr(u32::from(right & 0x0F))),
                    BinaryOp::And => Some(left & right),
                    BinaryOp::Or => Some(left | right),
                    BinaryOp::Xor => Some(left ^ right),
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge => None,
                }
            }
            _ => None,
        }?;
        Some(
            if !storage_address && expr.ty.value_width_bytes() == Some(1) {
                value & 0x00FF
            } else {
                value
            },
        )
    }

    fn const_u16_lvalue(&self, lvalue: &SemLValue) -> Option<u16> {
        if let Some(storage) = &lvalue.storage
            && matches!(
                storage.space,
                crate::semantic::ir::SemAddressSpace::Absolute
                    | crate::semantic::ir::SemAddressSpace::ZeroPage
                    | crate::semantic::ir::SemAddressSpace::RuntimeZeroPage
            )
            && let Some(address) = storage.address
        {
            return Some(address.wrapping_add(storage.offset));
        }
        match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => {
                self.semantic_absolute_globals.get(&symbol.id).copied()
            }
            _ => None,
        }
    }

    fn local_backing(
        &self,
        declaration: &SemDeclaration,
        record_storage_sizes: &BTreeMap<String, u16>,
        address_initializer: Option<u16>,
        local_alias_targets: &BTreeMap<SemSymbolId, (LocalId, String)>,
    ) -> NirLocalBacking {
        if let Some(address) = address_initializer {
            match &declaration.storage {
                SemDeclarationStorage::Scalar if !declaration.ty.value.pointer => {
                    return NirLocalBacking::Absolute(AddressValue::data(u64::from(address)));
                }
                SemDeclarationStorage::Array { .. }
                    if !declaration_array_address_initializer_uses_pointer_storage(
                        declaration,
                        record_storage_sizes,
                        self.target_layout,
                    ) =>
                {
                    return NirLocalBacking::Absolute(AddressValue::data(u64::from(address)));
                }
                _ => {}
            }
        }
        if let Some((target, target_name, offset)) =
            local_scalar_storage_alias_initializer(declaration, local_alias_targets)
        {
            return NirLocalBacking::Alias {
                target,
                target_name,
                offset: ByteOffset::from(offset),
            };
        }
        if matches!(declaration.storage, SemDeclarationStorage::Scalar)
            && !declaration.ty.value.pointer
            && let Some(initializer) = declaration.initializer.as_ref()
            && let Some((target_symbol, offset)) = storage_alias_initializer_expr(initializer)
            && self.storage_symbol_ids.contains(&target_symbol.id)
            && let Some(target) = self.global_ids_by_symbol.get(&target_symbol.id).copied()
        {
            return NirLocalBacking::GlobalAlias {
                target,
                target_name: target_symbol.name.clone(),
                offset: ByteOffset::from(offset),
            };
        }
        NirLocalBacking::Ordinary
    }
}

fn apply_target_layout_to_program(program: &mut NirProgram) {
    let layout = program.target_layout;
    for global in &mut program.globals {
        if let Some(ty) = &mut global.ty {
            ty.apply_target_layout(layout);
        }
    }
    for static_data in &mut program.statics {
        static_data.ty.apply_target_layout(layout);
    }
    for routine in &mut program.routines {
        for param in &mut routine.params {
            param.ty.apply_target_layout(layout);
        }
        for local in &mut routine.locals {
            local.ty.apply_target_layout(layout);
        }
        for temp in &mut routine.temps {
            temp.ty.apply_target_layout(layout);
        }
        for block in &mut routine.blocks {
            for param in &mut block.params {
                param.ty.apply_target_layout(layout);
            }
            for op in &mut block.ops {
                apply_target_layout_to_op(op, layout);
            }
            apply_target_layout_to_terminator(&mut block.terminator, layout);
        }
    }
}

fn apply_target_layout_to_op(op: &mut NirOp, layout: TargetLayout) {
    match op {
        NirOp::Load { ty, place, .. } | NirOp::VolatileLoad { ty, place, .. } => {
            ty.apply_target_layout(layout);
            apply_target_layout_to_place(place, layout);
        }
        NirOp::AddrOf { ty, place, .. } => {
            ty.apply_target_layout(layout);
            apply_target_layout_to_place(place, layout);
        }
        NirOp::Store { place, src, ty } | NirOp::VolatileStore { place, src, ty } => {
            apply_target_layout_to_place(place, layout);
            apply_target_layout_to_value(src, layout);
            ty.apply_target_layout(layout);
        }
        NirOp::CopyBytes {
            destination,
            source,
            ..
        } => {
            apply_target_layout_to_place(destination, layout);
            apply_target_layout_to_place(source, layout);
        }
        NirOp::Unary { ty, src, .. } => {
            ty.apply_target_layout(layout);
            apply_target_layout_to_value(src, layout);
        }
        NirOp::Cast {
            src, from, to, ..
        } => {
            apply_target_layout_to_value(src, layout);
            from.apply_target_layout(layout);
            to.apply_target_layout(layout);
        }
        NirOp::PointerOffset {
            ty, base, offset, ..
        } => {
            ty.apply_target_layout(layout);
            apply_target_layout_to_value(base, layout);
            apply_target_layout_to_value(offset, layout);
        }
        NirOp::Binary {
            ty, left, right, ..
        } => {
            ty.apply_target_layout(layout);
            apply_target_layout_to_value(left, layout);
            apply_target_layout_to_value(right, layout);
        }
        NirOp::Compare {
            ty,
            operand_ty,
            left,
            right,
            ..
        } => {
            ty.apply_target_layout(layout);
            operand_ty.apply_target_layout(layout);
            apply_target_layout_to_value(left, layout);
            apply_target_layout_to_value(right, layout);
        }
        NirOp::Real(real) => apply_target_layout_to_real_op(real, layout),
        NirOp::Call {
            callee,
            args,
            result,
            signature,
            ..
        } => {
            if let NirCallee::Indirect { target, ty } = callee {
                apply_target_layout_to_value(target, layout);
                ty.apply_target_layout(layout);
            }
            for arg in args {
                apply_target_layout_to_value(arg, layout);
            }
            if let Some(result) = result {
                result.ty.apply_target_layout(layout);
            }
            if let Some(signature) = signature {
                for param in &mut signature.params {
                    param.apply_target_layout(layout);
                }
                if let Some(variadic) = &mut signature.variadic {
                    variadic.apply_target_layout(layout);
                }
                if let Some(result) = &mut signature.result {
                    result.apply_target_layout(layout);
                }
            }
        }
        NirOp::RuntimeHelperOverride { .. }
        | NirOp::MachineBlock { .. }
        | NirOp::InlineAsm { .. }
        | NirOp::Unsupported { .. } => {}
    }
}

fn apply_target_layout_to_real_op(op: &mut NirRealOp, layout: TargetLayout) {
    let source = |source: &mut NirRealSource| {
        if let NirRealSource::Place(place) = source {
            apply_target_layout_to_place(place, layout);
        }
    };
    match op {
        NirRealOp::Copy {
            destination,
            source: operand,
        }
        | NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            apply_target_layout_to_place(destination, layout);
            source(operand);
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            apply_target_layout_to_place(destination, layout);
            source(left);
            source(right);
        }
        NirRealOp::Compare {
            result_type,
            left,
            right,
            ..
        } => {
            result_type.apply_target_layout(layout);
            source(left);
            source(right);
        }
        NirRealOp::IntegerToReal {
            destination,
            source: value,
            source_type,
        } => {
            apply_target_layout_to_place(destination, layout);
            apply_target_layout_to_value(value, layout);
            source_type.apply_target_layout(layout);
        }
        NirRealOp::RealToInteger {
            result_type,
            source,
            ..
        } => {
            result_type.apply_target_layout(layout);
            apply_target_layout_to_place(source, layout);
        }
    }
}

fn apply_target_layout_to_place(place: &mut NirPlace, layout: TargetLayout) {
    if let Some(ty) = &mut place.ty {
        ty.apply_target_layout(layout);
    }
    match &mut place.kind {
        NirPlaceKind::Deref { addr } => apply_target_layout_to_value(addr, layout),
        NirPlaceKind::Index {
            base_addr,
            index,
            elem_ty,
            ..
        } => {
            apply_target_layout_to_value(base_addr, layout);
            apply_target_layout_to_value(index, layout);
            elem_ty.apply_target_layout(layout);
        }
        NirPlaceKind::Field { base, ty, .. } => {
            apply_target_layout_to_place(base, layout);
            ty.apply_target_layout(layout);
        }
        NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_) => {}
    }
}

fn apply_target_layout_to_value(value: &mut NirValue, layout: TargetLayout) {
    match value {
        NirValue::AddressConst { address, ty } => {
            ty.apply_target_layout(layout);
            if let Some(address_space) = pointer_address_space(ty) {
                address.address_space = address_space;
            }
        }
        NirValue::Null { ty }
        | NirValue::StaticAddr { ty, .. }
        | NirValue::Temp { ty, .. }
        | NirValue::RoutineAddr { ty, .. } => ty.apply_target_layout(layout),
        NirValue::ConstU8(_)
        | NirValue::ConstU16(_)
        | NirValue::Param(_)
        | NirValue::GlobalAddr(_) => {}
    }
}

fn apply_target_layout_to_terminator(terminator: &mut NirTerminator, layout: TargetLayout) {
    let edge = |edge: &mut NirEdge| {
        for arg in &mut edge.args {
            apply_target_layout_to_value(arg, layout);
        }
    };
    match terminator {
        NirTerminator::Goto(target) => edge(target),
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            apply_target_layout_to_value(condition, layout);
            edge(then_edge);
            edge(else_edge);
        }
        NirTerminator::Return(Some(value)) => apply_target_layout_to_value(value, layout),
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(None)
        | NirTerminator::Exit => {}
    }
}

fn deduplicate_real_statics(statics: &mut Vec<NirStaticData>, routines: &mut [NirRoutine]) {
    let mut canonical = BTreeMap::<Vec<u8>, (SymbolId, String)>::new();
    let mut replacements = BTreeMap::<SymbolId, (SymbolId, String)>::new();
    statics.retain(|static_data| {
        if !is_real_nir_type(&static_data.ty) {
            return true;
        }
        match canonical.get(&static_data.image.bytes) {
            Some((id, name)) => {
                replacements.insert(static_data.id, (*id, name.clone()));
                false
            }
            None => {
                canonical.insert(
                    static_data.image.bytes.clone(),
                    (static_data.id, static_data.name.clone()),
                );
                true
            }
        }
    });
    if replacements.is_empty() {
        return;
    }
    for routine in routines {
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                let NirOp::Real(real) = op else {
                    continue;
                };
                match real {
                    NirRealOp::Copy { source, .. } => {
                        rewrite_real_static_source(source, &replacements);
                    }
                    NirRealOp::Unary { operand, .. } => {
                        rewrite_real_static_source(operand, &replacements);
                    }
                    NirRealOp::Binary { left, right, .. }
                    | NirRealOp::Compare { left, right, .. } => {
                        rewrite_real_static_source(left, &replacements);
                        rewrite_real_static_source(right, &replacements);
                    }
                    NirRealOp::IntegerToReal { .. } | NirRealOp::RealToInteger { .. } => {}
                }
            }
        }
    }
}

fn rewrite_real_static_source(
    source: &mut NirRealSource,
    replacements: &BTreeMap<SymbolId, (SymbolId, String)>,
) {
    let NirRealSource::Static { id, name } = source else {
        return;
    };
    if let Some((replacement_id, replacement_name)) = replacements.get(id) {
        *id = *replacement_id;
        *name = replacement_name.clone();
    }
}

fn local_scalar_storage_alias_initializer(
    declaration: &SemDeclaration,
    local_alias_targets: &BTreeMap<SemSymbolId, (LocalId, String)>,
) -> Option<(LocalId, String, u16)> {
    if !matches!(declaration.storage, SemDeclarationStorage::Scalar) || declaration.ty.value.pointer
    {
        return None;
    }
    let initializer = declaration.initializer.as_ref()?;
    let (target, offset) = storage_alias_initializer_expr(initializer)?;
    let (target_id, target_name) = local_alias_targets.get(&target.id)?;
    Some((*target_id, target_name.clone(), offset))
}

fn collect_nested_declarations<'a>(
    statements: &'a [SemStmt],
    declarations: &mut Vec<&'a SemDeclaration>,
) {
    for statement in statements {
        match statement {
            SemStmt::LexicalBlock {
                declarations: nested,
                body,
                ..
            } => {
                declarations.extend(nested);
                collect_nested_declarations(body, declarations);
            }
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    collect_nested_declarations(&branch.body, declarations);
                }
                collect_nested_declarations(else_body, declarations);
            }
            SemStmt::While { body, .. }
            | SemStmt::DoUntil { body, .. }
            | SemStmt::For { body, .. } => collect_nested_declarations(body, declarations),
            SemStmt::Define(_)
            | SemStmt::Return { .. }
            | SemStmt::Exit { .. }
            | SemStmt::Assign { .. }
            | SemStmt::RecordCopy { .. }
            | SemStmt::CompoundAssign { .. }
            | SemStmt::Call { .. }
            | SemStmt::MachineBlock { .. }
            | SemStmt::InlineAsm { .. }
            | SemStmt::Unsupported { .. } => {}
        }
    }
}

pub(super) struct NirBuilder {
    name: String,
    target_layout: TargetLayout,
    params: Vec<NirParam>,
    locals: Vec<NirLocal>,
    global_ids: BTreeMap<String, SymbolId>,
    global_ids_by_symbol: BTreeMap<SemSymbolId, SymbolId>,
    routine_ids: BTreeMap<String, u32>,
    routine_types: BTreeMap<String, NirType>,
    symbol_storage_types: BTreeMap<String, NirType>,
    semantic_storage_types: BTreeMap<SemSymbolId, NirType>,
    semantic_absolute_array_element_bases: BTreeMap<SemSymbolId, u16>,
    semantic_absolute_array_value_addresses: BTreeMap<SemSymbolId, u16>,
    param_ids_by_symbol: BTreeMap<SemSymbolId, ParamId>,
    local_ids_by_symbol: BTreeMap<SemSymbolId, LocalId>,
    record_storage_sizes: BTreeMap<String, u16>,
    machine_defines: BTreeMap<usize, Vec<MachineItem>>,
    machine_define_names: BTreeMap<String, Vec<MachineItem>>,
    notes: Vec<NirRoutineNote>,
    blocks: Vec<NirBlock>,
    block_ids: BTreeMap<String, BlockId>,
    current: usize,
    loop_exits: Vec<String>,
    next_block: u32,
    next_temp: u32,
    statics: Vec<NirStaticData>,
    next_static: u32,
}

impl NirBuilder {
    fn new(
        name: &str,
        entry_label: String,
        next_static: u32,
        global_ids: BTreeMap<String, SymbolId>,
        global_ids_by_symbol: BTreeMap<SemSymbolId, SymbolId>,
        routine_ids: BTreeMap<String, u32>,
        routine_types: BTreeMap<String, NirType>,
        symbol_storage_types: BTreeMap<String, NirType>,
        semantic_storage_types: BTreeMap<SemSymbolId, NirType>,
        semantic_absolute_array_element_bases: BTreeMap<SemSymbolId, u16>,
        semantic_absolute_array_value_addresses: BTreeMap<SemSymbolId, u16>,
        record_storage_sizes: BTreeMap<String, u16>,
        machine_defines: BTreeMap<usize, Vec<MachineItem>>,
        machine_define_names: BTreeMap<String, Vec<MachineItem>>,
        target_layout: TargetLayout,
    ) -> Self {
        let entry_id = BlockId(0);
        let block_ids = BTreeMap::from([(entry_label.clone(), entry_id)]);
        Self {
            name: name.to_string(),
            target_layout,
            params: Vec::new(),
            locals: Vec::new(),
            global_ids,
            global_ids_by_symbol,
            routine_ids,
            routine_types,
            symbol_storage_types,
            semantic_storage_types,
            semantic_absolute_array_element_bases,
            semantic_absolute_array_value_addresses,
            param_ids_by_symbol: BTreeMap::new(),
            local_ids_by_symbol: BTreeMap::new(),
            record_storage_sizes,
            machine_defines,
            machine_define_names,
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: entry_id,
                label: entry_label,
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Open,
            }],
            block_ids,
            current: 0,
            loop_exits: Vec::new(),
            next_block: 1,
            next_temp: 0,
            statics: Vec::new(),
            next_static,
        }
    }

    fn finish(self) -> (NirRoutine, Vec<NirStaticData>, u32) {
        (
            NirRoutine {
                name: self.name,
                params: self.params,
                locals: self.locals,
                temps: collect_temps(&self.blocks),
                notes: self.notes,
                blocks: self.blocks,
            },
            self.statics,
            self.next_static,
        )
    }

    fn push(&mut self, op: NirOp) {
        if !self.current_is_open() {
            let label = format!("{}.unreachable{}", self.name, self.blocks.len());
            self.start_block(label);
        }
        self.blocks[self.current].ops.push(op);
    }

    fn push_load(&mut self, dest: TempId, ty: NirType, place: NirPlace, is_volatile: bool) {
        self.push(if is_volatile {
            NirOp::VolatileLoad { dest, ty, place }
        } else {
            NirOp::Load { dest, ty, place }
        });
    }

    fn push_store(&mut self, place: NirPlace, src: NirValue, ty: NirType, is_volatile: bool) {
        self.push(if is_volatile {
            NirOp::VolatileStore { place, src, ty }
        } else {
            NirOp::Store { place, src, ty }
        });
    }

    fn stmt_list(&mut self, statements: &[SemStmt], lowering: &mut NirLowerer) {
        for stmt in statements {
            self.stmt(stmt, lowering);
        }
    }

    fn stmt(&mut self, stmt: &SemStmt, lowering: &mut NirLowerer) {
        match stmt {
            SemStmt::LexicalBlock { body, .. } => self.stmt_list(body, lowering),
            SemStmt::Define(_) => {}
            SemStmt::Return { value, .. } => {
                let value = value.as_ref().map(|value| self.nir_value(value));
                self.terminate(NirTerminator::Return(value));
            }
            SemStmt::Exit { .. } => {
                if let Some(label) = self.loop_exits.last().cloned() {
                    self.terminate_goto(&label);
                } else {
                    self.terminate(NirTerminator::Exit);
                }
            }
            SemStmt::Assign { target, value, .. } => {
                let is_volatile = target.is_volatile;
                let fallback_ty = NirFacts::type_from_value(&target.ty);
                let target = self.lower_place(target);
                let target_ty = target.ty.clone().unwrap_or(fallback_ty);
                if is_real_nir_type(&target_ty) {
                    self.lower_real_expr_into(value, target);
                    return;
                }
                let value = self.value(value);
                self.assign_or_store(target, target_ty, value, is_volatile);
            }
            SemStmt::RecordCopy {
                destination,
                source,
                size,
                ..
            } => {
                let destination_volatile = destination.is_volatile;
                let source_volatile = source.is_volatile;
                let destination = self.lower_place(destination);
                let source = self.lower_place(source);
                self.push(NirOp::CopyBytes {
                    destination,
                    source,
                    size: ByteSize::from(*size),
                    destination_volatile,
                    source_volatile,
                });
            }
            SemStmt::CompoundAssign {
                target, op, value, ..
            } => {
                let is_volatile = target.is_volatile;
                let fallback_ty = NirFacts::type_from_value(&target.ty);
                let target = self.lower_place(target);
                let target_ty = target.ty.clone().unwrap_or(fallback_ty);
                if is_real_nir_type(&target_ty) {
                    self.lower_real_compound_into(target, *op, value);
                    return;
                }
                let value = self.value(value);
                self.compound_or_unsupported(target, target_ty, *op, value, is_volatile);
            }
            SemStmt::Call { call, .. } => {
                if let Some(items) = self.machine_define_call_items(call) {
                    match items {
                        Ok(items) => self.push(NirOp::MachineBlock {
                            items,
                            effects: self.nir_machine_effects(&SemEffects::default()),
                        }),
                        Err(note) => self.push(NirOp::Unsupported { note }),
                    }
                    return;
                }
                let args = call.args.iter().map(|arg| self.nir_value(arg)).collect();
                let result = call.return_type.as_ref().map(|return_type| NirCallResult {
                    dest: self.next_temp(),
                    ty: NirFacts::type_from_value(return_type),
                });
                let callee = self.nir_callee(&call.callee);
                self.push(NirOp::Call {
                    callee,
                    args,
                    result,
                    signature: Some(nir_call_signature(call)),
                    effects: self.nir_call_effects(&call.effects),
                });
            }
            SemStmt::MachineBlock {
                items,
                resolved_symbols,
                effects,
                ..
            } => {
                if items.is_empty() {
                    return;
                }
                match self.nir_machine_items(items, resolved_symbols) {
                    Ok(items) => self.push(NirOp::MachineBlock {
                        items,
                        effects: self.nir_machine_effects(effects),
                    }),
                    Err(note) => self.push(NirOp::Unsupported { note }),
                }
            }
            SemStmt::InlineAsm { program, .. } => {
                if program.bytes.is_empty() {
                    return;
                }
                let code = self.nir_inline_asm(program);
                let effects = self.nir_inline_asm_effects(program, &code);
                self.push(NirOp::InlineAsm { code, effects });
            }
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                let after_label = lowering.next_block_label();
                for (index, branch) in branches.iter().enumerate() {
                    let body_label = lowering.next_block_label();
                    let next_label = if index + 1 == branches.len() && else_body.is_empty() {
                        after_label.clone()
                    } else {
                        lowering.next_block_label()
                    };
                    self.terminate_condition(&branch.condition, &body_label, &next_label, lowering);
                    self.start_block(body_label);
                    self.stmt_list(&branch.body, lowering);
                    self.finish_open_goto(&after_label);
                    self.start_block(next_label);
                }
                if !else_body.is_empty() {
                    self.stmt_list(else_body, lowering);
                    self.finish_open_goto(&after_label);
                }
                if self.current_label() != after_label {
                    self.start_block(after_label);
                }
            }
            SemStmt::While {
                condition, body, ..
            } => {
                let test_label = lowering.next_block_label();
                let body_label = lowering.next_block_label();
                let after_label = lowering.next_block_label();
                self.finish_open_goto(&test_label);
                self.start_block(test_label.clone());
                self.terminate_condition(condition, &body_label, &after_label, lowering);
                self.loop_exits.push(after_label.clone());
                self.start_block(body_label);
                self.stmt_list(body, lowering);
                self.finish_open_goto(&test_label);
                self.loop_exits.pop();
                self.start_block(after_label);
            }
            SemStmt::DoUntil {
                body, condition, ..
            } => {
                let body_label = lowering.next_block_label();
                let after_label = lowering.next_block_label();
                self.finish_open_goto(&body_label);
                self.loop_exits.push(after_label.clone());
                self.start_block(body_label.clone());
                self.stmt_list(body, lowering);
                if let Some(condition) = condition {
                    if self.current_is_open() {
                        self.terminate_condition(condition, &after_label, &body_label, lowering);
                    }
                } else {
                    self.finish_open_goto(&body_label);
                }
                self.loop_exits.pop();
                self.start_block(after_label);
            }
            SemStmt::For {
                target,
                start,
                end,
                step,
                step_control,
                body,
                ..
            } => {
                let is_volatile = target.is_volatile;
                let target_ty = NirFacts::type_from_value(&target.ty);
                let wrap_guard = match step_control {
                    SemForStep::Up(amount) => ascending_for_wrap_threshold(&target_ty, *amount)
                        .and_then(|threshold| {
                            let guard_is_unnecessary =
                                lowering.const_u16_expr(end).is_some_and(|bound| {
                                    for_bound_is_at_or_below(&target_ty, bound, threshold)
                                });
                            (!guard_is_unnecessary).then_some((NirCompareOp::Gt, threshold))
                        }),
                    SemForStep::Down(amount) => descending_for_wrap_threshold(&target_ty, *amount)
                        .and_then(|threshold| {
                            let guard_is_unnecessary =
                                lowering.const_u16_expr(end).is_some_and(|bound| {
                                    for_bound_is_at_or_above(&target_ty, bound, threshold)
                                });
                            (!guard_is_unnecessary).then_some((NirCompareOp::Lt, threshold))
                        }),
                    SemForStep::Unknown => None,
                };
                let target = self.lower_place(target);
                let test_label = lowering.next_block_label();
                let body_label = lowering.next_block_label();
                let after_label = lowering.next_block_label();
                let start = self.value(start);
                self.assign_or_store(target.clone(), target_ty.clone(), start, is_volatile);
                self.finish_open_goto(&test_label);
                self.start_block(test_label.clone());
                let compare = match step_control {
                    SemForStep::Down(_) => NirCompareOp::Ge,
                    SemForStep::Up(_) | SemForStep::Unknown => NirCompareOp::Le,
                };
                let condition = self.for_limit_condition(&target, end, compare, is_volatile);
                self.terminate_branch(condition, &body_label, &after_label);
                self.loop_exits.push(after_label.clone());
                self.start_block(body_label);
                self.stmt_list(body, lowering);
                if let Some((op, threshold)) = wrap_guard
                    && self.current_is_open()
                {
                    let step_label = lowering.next_block_label();
                    let condition =
                        self.for_wrap_condition(&target, &target_ty, op, threshold, is_volatile);
                    self.terminate_branch(condition, &after_label, &step_label);
                    self.start_block(step_label);
                }
                let value = step
                    .as_ref()
                    .map(|step| self.value(step))
                    .unwrap_or(Some(NirValue::ConstU8(1)));
                self.compound_or_unsupported(target, target_ty, BinaryOp::Add, value, is_volatile);
                self.finish_open_goto(&test_label);
                self.loop_exits.pop();
                self.start_block(after_label);
            }
            SemStmt::Unsupported { note, .. } => {
                self.push(NirOp::Unsupported { note: note.clone() })
            }
        }
    }

    fn machine_define_call_items(
        &self,
        call: &SemCall,
    ) -> Option<Result<Vec<NirMachineItem>, String>> {
        if !call.args.is_empty() {
            return None;
        }
        let SemCallable::Indirect { target, .. } = &call.callee else {
            return None;
        };
        let SemExprKind::Symbol(symbol) = &target.kind else {
            return None;
        };
        if symbol.class != SymbolClass::Define {
            return None;
        }
        self.machine_defines
            .get(&symbol.id.0)
            .map(|items| items.iter().map(nir_machine_item).collect())
    }

    fn nir_machine_items(
        &self,
        items: &[MachineItem],
        resolved_symbols: &[SemMachineSymbolRef],
    ) -> Result<Vec<NirMachineItem>, String> {
        let resolved_symbols = resolved_symbols
            .iter()
            .map(|resolved| (resolved.item_index, &resolved.symbol))
            .collect::<BTreeMap<_, _>>();
        let mut lowered = Vec::new();
        let mut index = 0;
        while index < items.len() {
            let item = &items[index];
            if let Some((byte, split_item)) =
                self.split_compact_machine_number_item(item, items.get(index + 1))
            {
                lowered.push(NirMachineItem::Byte(byte));
                if let Some(symbol) = resolved_symbols.get(&(index + 1))
                    && machine_symbol_has_link_identity(symbol)
                    && let Some(item) = self.resolved_nir_machine_symbol_item(&split_item, symbol)
                {
                    lowered.push(item);
                } else {
                    lowered.push(split_item);
                }
                index += 2;
                continue;
            }
            if matches!(item, MachineItem::Name(_))
                && let Some(symbol) = resolved_symbols.get(&index)
                && symbol.class == SymbolClass::Define
                && let Some(items) = self.machine_defines.get(&symbol.id.0)
            {
                lowered.extend(
                    items
                        .iter()
                        .map(nir_machine_item)
                        .collect::<Result<Vec<_>, _>>()?,
                );
                index += 1;
                continue;
            }
            if let MachineItem::Name(name) = item
                && let Some(items) = self.machine_define_names.get(&storage_key(name))
            {
                lowered.extend(
                    items
                        .iter()
                        .map(nir_machine_item)
                        .collect::<Result<Vec<_>, _>>()?,
                );
                index += 1;
                continue;
            }
            if let Some(symbol) = resolved_symbols.get(&index)
                && machine_symbol_has_link_identity(symbol)
                && let Some(item) = self.nir_machine_symbol_item(item, symbol)
            {
                lowered.push(item);
                index += 1;
                continue;
            }
            lowered.push(nir_machine_item(item)?);
            index += 1;
        }
        Ok(lowered)
    }

    fn nir_machine_symbol_item(
        &self,
        item: &MachineItem,
        symbol: &SemSymbolRef,
    ) -> Option<NirMachineItem> {
        let target = self.nir_inline_asm_symbol_target(symbol)?;
        let (kind, addend) = match item {
            MachineItem::Name(_) => (InlineAsmRelocationKind::Absolute16, 0),
            MachineItem::AddressByte { selector, .. } => (
                match selector {
                    AddressByteSelector::Low => InlineAsmRelocationKind::Low8,
                    AddressByteSelector::High => InlineAsmRelocationKind::High8,
                },
                0,
            ),
            MachineItem::AddressExpr(expr) => (
                match expr.selector {
                    Some(AddressByteSelector::Low) => InlineAsmRelocationKind::Low8,
                    Some(AddressByteSelector::High) => InlineAsmRelocationKind::High8,
                    None => InlineAsmRelocationKind::Absolute16,
                },
                expr.offset,
            ),
            MachineItem::Number(_)
            | MachineItem::StringLiteral(_)
            | MachineItem::CharLiteral(_)
            | MachineItem::Raw(_) => return None,
        };
        Some(NirMachineItem::Relocation {
            kind,
            target,
            addend,
            requires_zero_page: false,
            span: symbol.span,
        })
    }

    fn resolved_nir_machine_symbol_item(
        &self,
        item: &NirMachineItem,
        symbol: &SemSymbolRef,
    ) -> Option<NirMachineItem> {
        let target = self.nir_inline_asm_symbol_target(symbol)?;
        let (kind, addend) = match item {
            NirMachineItem::Name(_) => (InlineAsmRelocationKind::Absolute16, 0),
            NirMachineItem::AddressByte { high, .. } => (
                if *high {
                    InlineAsmRelocationKind::High8
                } else {
                    InlineAsmRelocationKind::Low8
                },
                0,
            ),
            NirMachineItem::AddressExpr {
                selector, offset, ..
            } => (
                match selector {
                    Some(NirMachineByteSelector::Low) => InlineAsmRelocationKind::Low8,
                    Some(NirMachineByteSelector::High) => InlineAsmRelocationKind::High8,
                    None => InlineAsmRelocationKind::Absolute16,
                },
                *offset,
            ),
            NirMachineItem::Byte(_)
            | NirMachineItem::Word(_)
            | NirMachineItem::StringLiteral(_)
            | NirMachineItem::CharLiteral(_)
            | NirMachineItem::Relocation { .. } => return None,
        };
        Some(NirMachineItem::Relocation {
            kind,
            target,
            addend,
            requires_zero_page: false,
            span: symbol.span,
        })
    }

    fn split_compact_machine_number_item(
        &self,
        item: &MachineItem,
        next: Option<&MachineItem>,
    ) -> Option<(u8, NirMachineItem)> {
        let MachineItem::Number(number) = item else {
            return None;
        };
        let digits = number.text.strip_prefix('$')?;
        if digits.len() <= 2 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let byte = u8::from_str_radix(&digits[..2], 16).ok()?;
        match next? {
            MachineItem::Name(suffix) => {
                let name = format!("{}{suffix}", &digits[2..]);
                self.machine_symbol_name_is_known(&name)
                    .then_some((byte, NirMachineItem::Name(name)))
            }
            MachineItem::AddressExpr(expr) => {
                let MachineAddressAtom::Name(suffix) = &expr.atom else {
                    return None;
                };
                let name = format!("{}{suffix}", &digits[2..]);
                self.machine_symbol_name_is_known(&name).then(|| {
                    (
                        byte,
                        NirMachineItem::AddressExpr {
                            selector: expr.selector.map(nir_machine_byte_selector),
                            explicit_address: expr.explicit_address,
                            atom: NirMachineAtom::Name(name),
                            offset: expr.offset,
                            text: format!("{}{}", &digits[2..], expr.text),
                        },
                    )
                })
            }
            _ => None,
        }
    }

    fn machine_symbol_name_is_known(&self, name: &str) -> bool {
        let key = storage_key(name);
        self.machine_define_names.contains_key(&key)
            || resident_variable(name).is_some()
            || matches!(
                key.as_str(),
                "EOL" | "CR" | "ESC" | "ESCAPE" | "CLEAR" | "CLS" | "BREAK" | "ERROR"
            )
            || self
                .global_ids
                .keys()
                .any(|candidate| storage_key(candidate) == key)
    }

    fn assign_or_store(
        &mut self,
        target: NirPlace,
        target_ty: NirType,
        value: Option<NirValue>,
        is_volatile: bool,
    ) {
        if let Some(src) = value {
            self.push_store(target, src, target_ty, is_volatile);
        } else {
            self.push(NirOp::Unsupported {
                note: "assignment source is not materialized".to_string(),
            });
        }
    }

    fn lower_real_compound_into(&mut self, destination: NirPlace, op: BinaryOp, value: &SemExpr) {
        let Some(operation) = NirClassifier::binary_op(op) else {
            self.push(NirOp::Unsupported {
                note: "REAL compound assignment operator is not supported".to_string(),
            });
            return;
        };
        let left = self.allocate_hidden_real_local();
        self.push(NirOp::Real(NirRealOp::Copy {
            destination: left.clone(),
            source: NirRealSource::Place(destination.clone()),
        }));
        let right = self.real_source(value);
        self.push(NirOp::Real(NirRealOp::Binary {
            operation,
            destination,
            left: NirRealSource::Place(left),
            right,
        }));
    }

    fn lower_real_expr_into(&mut self, expr: &SemExpr, destination: NirPlace) {
        match &expr.kind {
            SemExprKind::Literal(SemLiteral::Real { source, value }) => {
                let source = self.intern_real_literal(&source.text, *value);
                self.push(NirOp::Real(NirRealOp::Copy {
                    destination,
                    source,
                }));
            }
            SemExprKind::LValue(lvalue) => {
                let source = self.lower_place(lvalue);
                self.push(NirOp::Real(NirRealOp::Copy {
                    destination,
                    source: NirRealSource::Place(source),
                }));
            }
            SemExprKind::Symbol(symbol) => {
                let source =
                    self.resolved_symbol_place(symbol, Some(NirFacts::type_from_value(&expr.ty)));
                self.push(NirOp::Real(NirRealOp::Copy {
                    destination,
                    source: NirRealSource::Place(source),
                }));
            }
            SemExprKind::Cast { expr: inner, .. } if is_real_value_type(&inner.ty) => {
                self.lower_real_expr_into(inner, destination);
            }
            SemExprKind::Cast { expr: inner, .. } => {
                let source = self.nir_value(inner);
                self.push(NirOp::Real(NirRealOp::IntegerToReal {
                    destination,
                    source,
                    source_type: NirFacts::type_from_value(&inner.ty),
                }));
            }
            SemExprKind::Unary { op, expr: operand } => {
                let Some(operation) = NirClassifier::unary_op(*op) else {
                    self.push(NirOp::Unsupported {
                        note: "REAL unary operator is not supported".to_string(),
                    });
                    return;
                };
                let operand = self.real_source(operand);
                self.push(NirOp::Real(NirRealOp::Unary {
                    operation,
                    destination,
                    operand,
                }));
            }
            SemExprKind::Binary { op, left, right } => {
                let Some(operation) = NirClassifier::binary_op(*op) else {
                    self.push(NirOp::Unsupported {
                        note: "REAL binary operator is not supported".to_string(),
                    });
                    return;
                };
                let left = self.real_source(left);
                let right = self.real_source(right);
                self.push(NirOp::Real(NirRealOp::Binary {
                    operation,
                    destination,
                    left,
                    right,
                }));
            }
            _ => self.push(NirOp::Unsupported {
                note: "REAL expression is not materialized".to_string(),
            }),
        }
    }

    fn real_place(&mut self, expr: &SemExpr) -> NirPlace {
        let place = self.allocate_hidden_real_local();
        self.lower_real_expr_into(expr, place.clone());
        place
    }

    fn real_source(&mut self, expr: &SemExpr) -> NirRealSource {
        match &expr.kind {
            SemExprKind::Literal(SemLiteral::Real { source, value }) => {
                self.intern_real_literal(&source.text, *value)
            }
            SemExprKind::Cast { expr: inner, .. } if is_real_value_type(&inner.ty) => {
                self.real_source(inner)
            }
            _ => NirRealSource::Place(self.real_place(expr)),
        }
    }

    fn allocate_hidden_real_local(&mut self) -> NirPlace {
        let id = LocalId(
            self.locals
                .iter()
                .map(|local| local.id.0)
                .max()
                .map_or(0, |id| id.saturating_add(1)),
        );
        let mut name = format!("__nir_real_tmp_{}", id.0);
        while self.locals.iter().any(|local| local.name == name) {
            name.push('_');
        }
        let ty = real_nir_type();
        self.locals.push(NirLocal {
            id,
            name: name.clone(),
            kind: "hidden REAL evaluation".to_string(),
            purpose: NirLocalPurpose::RealTemporary,
            storage: NirStorageClass::Scalar,
            ty: ty.clone(),
            backing: NirLocalBacking::Ordinary,
            init: None,
        });
        NirPlace {
            kind: NirPlaceKind::Local { id, name },
            ty: Some(ty),
        }
    }

    fn intern_real_literal(
        &mut self,
        source: &str,
        value: crate::atari_real::AtariReal,
    ) -> NirRealSource {
        let bytes = value.to_bytes();
        if let Some(existing) = self.statics.iter().find(|static_data| {
            is_real_nir_type(&static_data.ty) && static_data.image.bytes == bytes
        }) {
            return NirRealSource::Static {
                id: existing.id,
                name: existing.name.clone(),
            };
        }
        let id = SymbolId(self.next_static);
        self.next_static += 1;
        let name = format!("__nir_real_{}_{}", sanitize_static_owner(&self.name), id.0);
        self.statics.push(NirStaticData {
            id,
            name: name.clone(),
            ty: real_nir_type(),
            image: NirDataImage::literal(bytes.to_vec()),
            display: source.to_string(),
            alignment: ByteSize::ONE,
            mutable: false,
            section: "rodata".to_string(),
        });
        NirRealSource::Static { id, name }
    }

    fn compound_or_unsupported(
        &mut self,
        target: NirPlace,
        target_ty: NirType,
        op: BinaryOp,
        value: Option<NirValue>,
        is_volatile: bool,
    ) {
        let Some(src) = value else {
            self.push(NirOp::Unsupported {
                note: "compound assignment source is not materialized".to_string(),
            });
            return;
        };
        let Some(op) = NirClassifier::binary_op(op) else {
            self.push(NirOp::Unsupported {
                note: "compound assignment operator is not supported".to_string(),
            });
            return;
        };

        let loaded = self.next_temp();
        self.push_load(loaded, target_ty.clone(), target.clone(), is_volatile);

        let result = self.next_temp();
        self.push(NirOp::Binary {
            dest: result,
            ty: target_ty.clone(),
            op,
            left: NirValue::Temp {
                id: loaded,
                ty: target_ty.clone(),
            },
            right: src,
        });

        self.push_store(
            target,
            NirValue::Temp {
                id: result,
                ty: target_ty.clone(),
            },
            target_ty,
            is_volatile,
        );
    }

    fn value(&mut self, expr: &SemExpr) -> Option<NirValue> {
        match &expr.kind {
            SemExprKind::Binary { op, left, right }
                if NirClassifier::is_nir_compare_op(*op)
                    && (is_real_value_type(&left.ty) || is_real_value_type(&right.ty)) =>
            {
                let left = self.real_source(left);
                let right = self.real_source(right);
                let result = self.next_temp();
                self.push(NirOp::Real(NirRealOp::Compare {
                    predicate: NirClassifier::compare_op(*op)
                        .expect("compare-classified REAL op should lower to NIR"),
                    result,
                    result_type: NirFacts::condition_type(),
                    left,
                    right,
                }));
                Some(NirValue::Temp {
                    id: result,
                    ty: NirFacts::condition_type(),
                })
            }
            SemExprKind::Binary { op, left, right } if NirClassifier::is_nir_compare_op(*op) => {
                let operand_ty = NirFacts::type_from_value(&left.ty);
                let left = self.nir_value(left);
                let right = self.nir_value(right);
                let dest = self.next_temp();
                let ty = NirFacts::condition_type();
                self.push(NirOp::Compare {
                    dest,
                    ty: ty.clone(),
                    operand_ty,
                    op: NirClassifier::compare_op(*op)
                        .expect("compare-classified op should lower to NIR"),
                    left,
                    right,
                });
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::Cast { expr: inner, .. }
                if is_real_value_type(&inner.ty) && !is_real_value_type(&expr.ty) =>
            {
                let source = self.real_place(inner);
                let result = self.next_temp();
                let result_type = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Real(NirRealOp::RealToInteger {
                    result,
                    result_type: result_type.clone(),
                    source,
                }));
                Some(NirValue::Temp {
                    id: result,
                    ty: result_type,
                })
            }
            SemExprKind::Cast { expr: inner, .. } => {
                let src = self.nir_value(inner);
                let from = NirFacts::type_from_value(&inner.ty);
                let to = NirFacts::type_from_value(&expr.ty);
                let kind = nir_cast_kind(&from, &to);
                if to.kind.is_address() {
                    let value = match &src {
                        NirValue::ConstU8(value) => Some(u64::from(*value)),
                        NirValue::ConstU16(value) => Some(u64::from(*value)),
                        _ => None,
                    };
                    if let Some(value) = value {
                        return Some(if value == 0 {
                            NirValue::Null { ty: to }
                        } else {
                            NirValue::AddressConst {
                                address: AddressValue::new(
                                    pointer_address_space(&to)
                                        .expect("address type must name an address space"),
                                    value,
                                ),
                                ty: to,
                            }
                        });
                    }
                }
                let dest = self.next_temp();
                self.push(NirOp::Cast {
                    dest,
                    src,
                    from,
                    to: to.clone(),
                    kind,
                });
                Some(NirValue::Temp { id: dest, ty: to })
            }
            SemExprKind::Unary { op, expr: inner } if NirClassifier::unary_op(*op).is_some() => {
                let src = self.nir_value(inner);
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Unary {
                    dest,
                    ty: ty.clone(),
                    op: NirClassifier::unary_op(*op)
                        .expect("unary-classified op should lower to NIR"),
                    src,
                });
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::Binary { op, left, right } => {
                let left_ty = NirFacts::type_from_value(&left.ty);
                let right_ty = NirFacts::type_from_value(&right.ty);
                let left = self.nir_value(left);
                let right = self.nir_value(right);
                let dest = self.next_temp();
                let mut ty = NirFacts::type_from_value(&expr.ty);
                let operation = NirClassifier::binary_op(*op)
                    .expect("binary expression op should lower to NIR");
                let pointer_operands = match operation {
                    NirBinaryOp::Add if left_ty.kind.is_pointer() && !right_ty.kind.is_address() => {
                        Some((left.clone(), right.clone(), left_ty))
                    }
                    NirBinaryOp::Add if right_ty.kind.is_pointer() && !left_ty.kind.is_address() => {
                        Some((right.clone(), left.clone(), right_ty))
                    }
                    NirBinaryOp::Sub if left_ty.kind.is_pointer() && !right_ty.kind.is_address() => {
                        Some((left.clone(), right.clone(), left_ty))
                    }
                    _ => None,
                };
                if let Some((base, offset, pointer_ty)) = pointer_operands {
                    ty = pointer_ty;
                    self.push(NirOp::PointerOffset {
                        dest,
                        ty: ty.clone(),
                        base,
                        offset,
                        subtract: operation == NirBinaryOp::Sub,
                    });
                } else {
                    self.push(NirOp::Binary {
                        dest,
                        ty: ty.clone(),
                        op: operation,
                        left,
                        right,
                    });
                }
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::LValue(lvalue) => {
                let is_volatile = lvalue.is_volatile;
                let place = self.lower_place(lvalue);
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                self.push_load(dest, ty.clone(), place, is_volatile);
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::Symbol(symbol) => {
                let dest = self.next_temp();
                let ty = self
                    .symbol_storage_type(symbol)
                    .unwrap_or_else(|| NirFacts::type_from_value(&expr.ty));
                let place = self.resolved_symbol_place(symbol, Some(ty.clone()));
                self.push_load(dest, ty.clone(), place, symbol.is_volatile);
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::AddressOf(lvalue) => {
                let source_symbol = lvalue_symbol(lvalue);
                let place = self.lower_place(lvalue);
                let ty = NirFacts::type_from_value(&expr.ty);
                Some(self.addr_of_place(place, ty, source_symbol))
            }
            SemExprKind::AddressOfSymbol(symbol) => {
                if matches!(
                    symbol.class,
                    SymbolClass::Proc
                        | SymbolClass::Func
                        | SymbolClass::BuiltinProc
                        | SymbolClass::BuiltinFunc
                ) {
                    let id = *self
                        .routine_ids
                        .get(&storage_key(&symbol.name))
                        .expect("resolved routine address must have a routine id");
                    return Some(NirValue::RoutineAddr {
                        id,
                        name: symbol.name.clone(),
                        ty: symbol
                            .ty
                            .as_ref()
                            .map(NirType::from_value)
                            .filter(|ty| matches!(ty.kind, NirTypeKind::Callable { .. }))
                            .or_else(|| {
                                self.routine_types
                                    .get(&storage_key(&symbol.name))
                                    .cloned()
                            })
                            .unwrap_or_else(|| NirFacts::type_from_value(&expr.ty)),
                    });
                }
                let place_ty = symbol
                    .ty
                    .as_ref()
                    .map(NirType::from_value)
                    .or_else(|| Some(NirFacts::type_from_value(&expr.ty)));
                let place = self.resolved_symbol_place(symbol, place_ty);
                let ty = NirFacts::type_from_value(&expr.ty);
                Some(self.addr_of_place(place, ty, Some(symbol)))
            }
            SemExprKind::ImplicitAddressOf(address) => {
                let source_symbol = lvalue_symbol(&address.place);
                let place = self.lower_place(&address.place);
                let ty = NirFacts::type_from_value(&expr.ty);
                Some(self.addr_of_place(place, ty, source_symbol))
            }
            SemExprKind::ArrayDecay(decay) => {
                if decay.origin == SemArrayOrigin::Parameter
                    || self.lvalue_uses_pointer_storage(&decay.array)
                {
                    let place = self.lower_place(&decay.array);
                    let ty = NirFacts::type_from_value(&expr.ty);
                    return Some(self.load_place_value(place, ty));
                }
                let source_symbol = lvalue_symbol(&decay.array);
                let place = self.lower_place(&decay.array);
                let ty = NirFacts::type_from_value(&expr.ty);
                Some(self.addr_of_place(place, ty, source_symbol))
            }
            SemExprKind::Call(call) if NirClassifier::is_index_call_syntax(call) => {
                let is_volatile = matches!(
                    &call.callee,
                    SemCallable::User(symbol) if symbol.is_volatile
                );
                let place = self.lower_call_index_place(call, &expr.ty);
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                self.push_load(dest, ty.clone(), place, is_volatile);
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::Call(call) if NirClassifier::is_materializable_call(call) => {
                let args = call.args.iter().map(|arg| self.nir_value(arg)).collect();
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                let callee = self.nir_callee(&call.callee);
                self.push(NirOp::Call {
                    callee,
                    args,
                    result: Some(NirCallResult {
                        dest,
                        ty: ty.clone(),
                    }),
                    signature: Some(nir_call_signature(call)),
                    effects: self.nir_call_effects(&call.effects),
                });
                Some(NirValue::Temp { id: dest, ty })
            }
            SemExprKind::Literal(literal) => {
                literal_value(literal, &NirFacts::type_from_value(&expr.ty))
            }
            _ => None,
        }
    }

    fn nir_value(&mut self, expr: &SemExpr) -> NirValue {
        if let SemExprKind::Literal(SemLiteral::String(value)) = &expr.kind {
            return self.intern_string_literal(value, NirFacts::type_from_value(&expr.ty));
        }
        self.value(expr)
            .expect("lowered NIR value should be materialized")
    }

    fn intern_string_literal(&mut self, value: &str, ty: NirType) -> NirValue {
        let id = SymbolId(self.next_static);
        self.next_static += 1;
        let name = format!("__nir_str_{}_{}", sanitize_static_owner(&self.name), id.0);
        self.statics.push(NirStaticData {
            id,
            name: name.clone(),
            ty: ty.clone(),
            image: NirDataImage::literal(
                string_literal_storage_bytes(value).unwrap_or_else(|_| value.as_bytes().to_vec()),
            ),
            display: value.to_string(),
            alignment: ByteSize::ONE,
            mutable: false,
            section: "rodata".to_string(),
        });
        NirValue::StaticAddr { id, name, ty }
    }

    fn addr_of_place(
        &mut self,
        place: NirPlace,
        ty: NirType,
        source_symbol: Option<&SemSymbolRef>,
    ) -> NirValue {
        if let Some(symbol) = source_symbol
            && let Some(address) = self
                .semantic_absolute_array_value_addresses
                .get(&symbol.id)
                .copied()
                .or_else(|| resident_array_address(&symbol.name))
        {
            return NirValue::ConstU16(address);
        }
        let dest = self.next_temp();
        self.push(NirOp::AddrOf {
            dest,
            ty: ty.clone(),
            place,
        });
        NirValue::Temp { id: dest, ty }
    }

    fn load_place_value(&mut self, place: NirPlace, ty: NirType) -> NirValue {
        let dest = self.next_temp();
        self.push_load(dest, ty.clone(), place, false);
        NirValue::Temp { id: dest, ty }
    }

    fn lower_place(&mut self, lvalue: &SemLValue) -> NirPlace {
        let ty = self.lvalue_storage_type(lvalue);
        match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => {
                if let Some(address) = lvalue.storage.as_ref().and_then(|storage| storage.address) {
                    NirPlace {
                        kind: NirPlaceKind::Absolute(AddressValue::data(u64::from(address))),
                        ty,
                    }
                } else {
                    self.resolved_symbol_place(symbol, ty)
                }
            }
            SemLValueKind::UnresolvedName(name) => {
                panic!("unresolved storage name `{name}` must be diagnosed before NIR lowering")
            }
            SemLValueKind::Deref { pointer } => {
                let addr = self.nir_value(pointer);
                NirPlace {
                    kind: NirPlaceKind::Deref { addr },
                    ty,
                }
            }
            SemLValueKind::Index {
                base,
                index,
                element_type,
                ..
            } => NirPlace {
                kind: self.lower_index_place(base, index, element_type),
                ty,
            },
            SemLValueKind::Field { base, field } => NirPlace {
                kind: NirPlaceKind::Field {
                    base: Box::new(self.lower_place(base)),
                    offset: ByteOffset::from(field.offset.unwrap_or(0)),
                    ty: NirFacts::type_from_value(&field.ty),
                },
                ty,
            },
        }
    }

    fn resolved_symbol_place(&self, symbol: &SemSymbolRef, ty: Option<NirType>) -> NirPlace {
        if let Some(local_id) = self.local_ids_by_symbol.get(&symbol.id)
            && let Some(local) = self.locals.iter().find(|local| local.id == *local_id)
        {
            let kind = match &local.backing {
                NirLocalBacking::Absolute(address) => NirPlaceKind::Absolute(*address),
                NirLocalBacking::GlobalAlias {
                    target,
                    target_name,
                    offset,
                } => {
                    let ty = ty.clone().unwrap_or(NirType {
                        kind: NirTypeKind::U8,
                        summary: "Byte".to_string(),
                        width: Some(ByteSize::ONE),
                        pointer: false,
                    });
                    NirPlaceKind::Field {
                        base: Box::new(NirPlace {
                            kind: NirPlaceKind::Global {
                                id: *target,
                                name: target_name.clone(),
                            },
                            ty: Some(ty.clone()),
                        }),
                        offset: *offset,
                        ty,
                    }
                }
                NirLocalBacking::Ordinary | NirLocalBacking::Alias { .. } => NirPlaceKind::Local {
                    id: local.id,
                    name: local.name.clone(),
                },
            };
            return NirPlace { kind, ty };
        }
        if let Some(param_id) = self.param_ids_by_symbol.get(&symbol.id)
            && let Some(param) = self.params.iter().find(|param| param.id == *param_id)
        {
            return NirPlace {
                kind: NirPlaceKind::Param {
                    id: param.id,
                    name: symbol.name.clone(),
                },
                ty,
            };
        }
        if let Some(id) = self.global_ids_by_symbol.get(&symbol.id).copied() {
            return NirPlace {
                kind: NirPlaceKind::Global {
                    id,
                    name: symbol.name.clone(),
                },
                ty,
            };
        }
        if let Some(address) = builtin_variable_address(&symbol.name) {
            return NirPlace {
                kind: NirPlaceKind::Absolute(AddressValue::data(u64::from(address))),
                ty,
            };
        }
        panic!(
            "resolved storage symbol `{}#{}` has no NIR storage identity",
            symbol.name, symbol.id.0
        )
    }

    fn lvalue_storage_type(&self, lvalue: &SemLValue) -> Option<NirType> {
        if let SemLValueKind::Symbol(symbol) = &lvalue.kind
            && let Some(ty) = self.symbol_storage_type(symbol)
        {
            return Some(ty);
        }
        Some(self.storage_type_for_value(&lvalue.ty))
    }

    fn storage_type_for_value(&self, value: &ValueType) -> NirType {
        let mut ty = NirType::from_value_with_layout(value, self.target_layout);
        if let NirTypeKind::Record { name, size } = &mut ty.kind
            && let Some(storage_size) = self.record_storage_sizes.get(name).copied()
        {
            *size = Some(ByteSize::from(storage_size));
            ty.width = Some(ByteSize::from(storage_size));
        }
        ty
    }

    fn symbol_storage_type(&self, symbol: &SemSymbolRef) -> Option<NirType> {
        self.semantic_storage_types
            .get(&symbol.id)
            .cloned()
            .or_else(|| builtin_variable_type(&symbol.name))
    }

    fn lvalue_uses_pointer_storage(&self, lvalue: &SemLValue) -> bool {
        matches!(
            &lvalue.kind,
            SemLValueKind::Symbol(symbol) if self.semantic_storage_types.contains_key(&symbol.id)
        )
    }

    fn lower_index_place(
        &mut self,
        base: &SemExpr,
        index: &SemExpr,
        element_type: &ValueType,
    ) -> NirPlaceKind {
        let elem_ty = NirFacts::type_from_value(element_type);
        let elem_size = self.element_width(element_type).unwrap_or(ByteSize::ONE);
        NirPlaceKind::Index {
            base_addr: self.index_base_addr(base, element_type),
            index: self.nir_value(index),
            elem_ty,
            elem_size,
        }
    }

    fn lower_call_index_place(&mut self, call: &SemCall, ty: &ValueType) -> NirPlace {
        let SemCallable::User(symbol) = &call.callee else {
            unreachable!("index call syntax is only formed from user symbols")
        };
        let index = call
            .args
            .first()
            .expect("index call syntax has one argument");
        let elem_ty = NirFacts::type_from_value(ty);
        let elem_size = self.element_width(ty).unwrap_or(ByteSize::ONE);
        let place = self.resolved_symbol_place(symbol, symbol.ty.as_ref().map(NirType::from_value));
        let pointer_ty = pointer_type_to(ty);
        let base_addr = if matches!(symbol.class, crate::semantic::SymbolClass::Param) {
            self.load_place_value(place, pointer_ty)
        } else if let Some(address) = self.absolute_index_base_for_symbol(symbol) {
            NirValue::ConstU16(address)
        } else {
            self.addr_of_place(place, pointer_ty, Some(symbol))
        };
        NirPlace {
            kind: NirPlaceKind::Index {
                base_addr,
                index: self.nir_value(index),
                elem_ty,
                elem_size,
            },
            ty: Some(NirFacts::type_from_value(ty)),
        }
    }

    fn element_width(&self, ty: &ValueType) -> Option<ByteSize> {
        ty.value_width_bytes_for_layout(self.target_layout)
            .map(ByteSize::from)
            .or_else(|| {
            ty.as_record_name()
                .and_then(|name| self.record_storage_sizes.get(name).copied())
                .map(ByteSize::from)
            })
    }

    fn index_base_addr(&mut self, base: &SemExpr, element_type: &ValueType) -> NirValue {
        if base.ty.pointer {
            return self.nir_value(base);
        }
        if let Some(address) = self.absolute_array_base_address(base) {
            return NirValue::ConstU16(address);
        }
        if let SemExprKind::LValue(lvalue) = &base.kind
            && lvalue_is_param_symbol(lvalue)
        {
            let place = self.lower_place(lvalue);
            let pointer_ty = pointer_type_to(element_type);
            return self.load_place_value(place, pointer_ty);
        }
        if let SemExprKind::Symbol(symbol) = &base.kind
            && matches!(symbol.class, crate::semantic::SymbolClass::Param)
        {
            let pointer_ty = pointer_type_to(element_type);
            let place = self.resolved_symbol_place(symbol, Some(pointer_ty.clone()));
            return self.load_place_value(place, pointer_ty);
        }

        let place = match &base.kind {
            SemExprKind::Symbol(symbol) => {
                self.resolved_symbol_place(symbol, symbol.ty.as_ref().map(NirType::from_value))
            }
            SemExprKind::LValue(lvalue) => self.lower_place(lvalue),
            _ => return self.nir_value(base),
        };
        let pointer_ty = pointer_type_to(element_type);
        self.addr_of_place(place, pointer_ty, None)
    }

    fn absolute_array_base_address(&self, base: &SemExpr) -> Option<u16> {
        let symbol = match &base.kind {
            SemExprKind::Symbol(symbol) => Some(symbol),
            SemExprKind::LValue(lvalue) => lvalue_symbol(lvalue),
            SemExprKind::ArrayDecay(decay) => lvalue_symbol(&decay.array),
            _ => None,
        }?;
        self.absolute_index_base_for_symbol(symbol)
    }

    fn absolute_index_base_for_symbol(&self, symbol: &SemSymbolRef) -> Option<u16> {
        self.semantic_absolute_array_value_addresses
            .get(&symbol.id)
            .or_else(|| self.semantic_absolute_array_element_bases.get(&symbol.id))
            .copied()
            .or_else(|| resident_array_address(&symbol.name))
    }

    fn condition(&mut self, condition: &SemCondition) -> NirValue {
        match condition.kind {
            SemConditionKind::ConstantFalse => NirValue::ConstU8(0),
            SemConditionKind::ConstantTrue => NirValue::ConstU8(1),
            SemConditionKind::Compare => self.nir_value(&condition.expr),
            SemConditionKind::Logical | SemConditionKind::NonZeroValue => {
                self.nonzero_condition(&condition.expr)
            }
            SemConditionKind::Error | SemConditionKind::Unknown => self.nir_value(&condition.expr),
        }
    }

    fn terminate_condition(
        &mut self,
        condition: &SemCondition,
        then_label: &str,
        else_label: &str,
        lowering: &mut NirLowerer,
    ) {
        if condition.kind == SemConditionKind::Logical {
            self.terminate_logical_condition_expr(
                &condition.expr,
                then_label,
                else_label,
                lowering,
            );
            return;
        }

        let condition = self.condition(condition);
        self.terminate_branch(condition, then_label, else_label);
    }

    fn terminate_logical_condition_expr(
        &mut self,
        expr: &SemExpr,
        then_label: &str,
        else_label: &str,
        lowering: &mut NirLowerer,
    ) {
        let SemExprKind::Binary {
            op: BinaryOp::And | BinaryOp::Or,
            left,
            right,
        } = &expr.kind
        else {
            let condition = self.logical_operand_condition(expr);
            self.terminate_branch(condition, then_label, else_label);
            return;
        };

        let right_label = lowering.next_block_label();
        match &expr.kind {
            SemExprKind::Binary {
                op: BinaryOp::And, ..
            } => self.terminate_logical_condition_expr(left, &right_label, else_label, lowering),
            SemExprKind::Binary {
                op: BinaryOp::Or, ..
            } => self.terminate_logical_condition_expr(left, then_label, &right_label, lowering),
            _ => unreachable!("logical expression root was checked above"),
        }
        self.start_block(right_label);
        self.terminate_logical_condition_expr(right, then_label, else_label, lowering);
    }

    fn logical_operand_condition(&mut self, expr: &SemExpr) -> NirValue {
        if expr.class == SemExprClass::Condition {
            self.nir_value(expr)
        } else {
            self.nonzero_condition(expr)
        }
    }

    fn nonzero_condition(&mut self, expr: &SemExpr) -> NirValue {
        if is_real_value_type(&expr.ty) {
            let left = self.real_source(expr);
            let zero = self.intern_real_literal("0", crate::atari_real::AtariReal::ZERO);
            let result = self.next_temp();
            let result_type = NirFacts::condition_type();
            self.push(NirOp::Real(NirRealOp::Compare {
                predicate: NirCompareOp::Ne,
                result,
                result_type: result_type.clone(),
                left,
                right: zero,
            }));
            return NirValue::Temp {
                id: result,
                ty: result_type,
            };
        }
        let value = self.nir_value(expr);
        match value {
            NirValue::ConstU8(value) => NirValue::ConstU8(u8::from(value != 0)),
            NirValue::ConstU16(value) => NirValue::ConstU8(u8::from(value != 0)),
            value => {
                let dest = self.next_temp();
                let ty = NirFacts::condition_type();
                let operand_ty = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Compare {
                    dest,
                    ty: ty.clone(),
                    operand_ty,
                    op: NirCompareOp::Ne,
                    left: value,
                    right: zero_value_for_type(&expr.ty),
                });
                NirValue::Temp { id: dest, ty }
            }
        }
    }

    fn for_limit_condition(
        &mut self,
        target: &NirPlace,
        end: &SemExpr,
        op: NirCompareOp,
        is_volatile: bool,
    ) -> NirValue {
        let left_ty = target.ty.clone().unwrap_or_else(NirFacts::condition_type);
        let left_temp = self.next_temp();
        self.push_load(left_temp, left_ty.clone(), target.clone(), is_volatile);
        let right = self.nir_value(end);
        let dest = self.next_temp();
        let ty = NirFacts::condition_type();
        self.push(NirOp::Compare {
            dest,
            ty: ty.clone(),
            operand_ty: left_ty.clone(),
            op,
            left: NirValue::Temp {
                id: left_temp,
                ty: left_ty,
            },
            right,
        });
        NirValue::Temp { id: dest, ty }
    }

    fn for_wrap_condition(
        &mut self,
        target: &NirPlace,
        target_ty: &NirType,
        op: NirCompareOp,
        threshold: u16,
        is_volatile: bool,
    ) -> NirValue {
        let left_temp = self.next_temp();
        self.push_load(left_temp, target_ty.clone(), target.clone(), is_volatile);
        let dest = self.next_temp();
        let ty = NirFacts::condition_type();
        self.push(NirOp::Compare {
            dest,
            ty: ty.clone(),
            operand_ty: target_ty.clone(),
            op,
            left: NirValue::Temp {
                id: left_temp,
                ty: target_ty.clone(),
            },
            right: nir_scalar_constant(target_ty, threshold),
        });
        NirValue::Temp { id: dest, ty }
    }

    fn next_temp(&mut self) -> TempId {
        let temp = TempId(self.next_temp);
        self.next_temp += 1;
        temp
    }

    fn nir_callee(&mut self, callable: &SemCallable) -> NirCallee {
        match callable {
            SemCallable::User(symbol) => NirCallee::User {
                id: *self
                    .routine_ids
                    .get(&storage_key(&symbol.name))
                    .expect("resolved user callee must have a routine id"),
                name: symbol.name.clone(),
            },
            SemCallable::Builtin(symbol) => {
                if let Some(id) = self.routine_ids.get(&storage_key(&symbol.name)) {
                    NirCallee::User {
                        id: *id,
                        name: symbol.name.clone(),
                    }
                } else {
                    NirCallee::Builtin(symbol.name.clone())
                }
            }
            SemCallable::Indirect { target, .. } => NirCallee::Indirect {
                target: self.nir_value(target),
                ty: NirFacts::type_from_value(&target.ty),
            },
            SemCallable::Runtime { name, address, .. } => NirCallee::Runtime {
                name: name.clone(),
                address: address.map(|address| AddressValue::code(u64::from(address))),
            },
        }
    }

    fn nir_call_effects(&self, effects: &SemEffects) -> NirCallEffects {
        NirCallEffects {
            memory: NirMemoryEffects {
                reads: self.nir_read_effects(&effects.reads, effects.opaque),
                writes: self.nir_write_effects(&effects.writes, effects.opaque),
            },
            may_call_os: effects.may_call_os,
            opaque: effects.opaque,
        }
    }

    fn nir_machine_effects(&self, effects: &SemEffects) -> NirMachineEffects {
        NirMachineEffects {
            memory: NirMemoryEffects {
                reads: self.nir_read_effects(&effects.reads, effects.opaque),
                writes: self.nir_write_effects(&effects.writes, effects.opaque),
            },
            may_call_os: effects.may_call_os,
            opaque: true,
        }
    }

    fn nir_inline_asm(&self, program: &SemInlineAsm) -> NirInlineAsm {
        let relocations = program
            .relocations
            .iter()
            .filter_map(|relocation| {
                let target = match &relocation.target {
                    SemInlineAsmTarget::InlineOffset(offset) => {
                        NirInlineAsmTarget::InlineOffset(ByteOffset::from(*offset))
                    }
                    SemInlineAsmTarget::Absolute(address) => NirInlineAsmTarget::Absolute(
                        AddressValue::data(u64::from(*address)),
                    ),
                    SemInlineAsmTarget::Symbol(symbol) => {
                        self.nir_inline_asm_symbol_target(symbol)?
                    }
                };
                Some(NirInlineAsmRelocation {
                    offset: ByteOffset::from(relocation.offset),
                    kind: relocation.kind,
                    target,
                    addend: relocation.addend,
                    requires_zero_page: relocation.requires_zero_page,
                    symbol_use: relocation.symbol_use,
                    span: relocation.span,
                })
            })
            .collect();
        NirInlineAsm {
            bytes: program.bytes.clone(),
            relocations,
            source: program.source.clone(),
        }
    }

    fn nir_inline_asm_symbol_target(
        &self,
        symbol: &crate::semantic::ir::SemSymbolRef,
    ) -> Option<NirInlineAsmTarget> {
        let key = storage_key(&symbol.name);
        match symbol.class {
            SymbolClass::Param => self
                .param_ids_by_symbol
                .get(&symbol.id)
                .copied()
                .map(|id| NirInlineAsmTarget::Storage(NirStorageId::Param(id))),
            SymbolClass::Var | SymbolClass::Array | SymbolClass::Record | SymbolClass::Type => {
                if let Some(local_id) = self.local_ids_by_symbol.get(&symbol.id)
                    && let Some(local) = self.locals.iter().find(|local| local.id == *local_id)
                {
                    return Some(match local.backing {
                        NirLocalBacking::Absolute(address) => NirInlineAsmTarget::Absolute(address),
                        NirLocalBacking::Ordinary
                        | NirLocalBacking::Alias { .. }
                        | NirLocalBacking::GlobalAlias { .. } => {
                            NirInlineAsmTarget::Storage(NirStorageId::Local(local.id))
                        }
                    });
                }
                if let Some(address) = self
                    .semantic_absolute_array_value_addresses
                    .get(&symbol.id)
                    .copied()
                {
                    return Some(NirInlineAsmTarget::Absolute(AddressValue::data(u64::from(
                        address,
                    ))));
                }
                self.global_ids_by_symbol
                    .get(&symbol.id)
                    .copied()
                    .map(|id| NirInlineAsmTarget::Storage(NirStorageId::Global(id)))
            }
            SymbolClass::Proc
            | SymbolClass::Func
            | SymbolClass::BuiltinProc
            | SymbolClass::BuiltinFunc => self
                .routine_ids
                .get(&key)
                .copied()
                .map(NirInlineAsmTarget::Routine),
            SymbolClass::Define => self
                .machine_defines
                .get(&symbol.id.0)
                .and_then(|items| match items.as_slice() {
                    [MachineItem::Number(number)] => number.value,
                    _ => None,
                })
                .map(|address| NirInlineAsmTarget::Absolute(AddressValue::data(u64::from(address)))),
            SymbolClass::Const => None,
        }
    }

    fn nir_inline_asm_effects(
        &self,
        program: &SemInlineAsm,
        code: &NirInlineAsm,
    ) -> NirMachineEffects {
        if program.mode == InlineAsmMode::Opaque {
            return NirMachineEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::Unknown,
                    writes: NirMemoryAccess::Unknown,
                },
                may_call_os: true,
                opaque: true,
            };
        }

        let reads_unknown = code.relocations.iter().any(|relocation| {
            matches!(
                relocation.symbol_use,
                InlineAsmSymbolUse::Call
                    | InlineAsmSymbolUse::Control
                    | InlineAsmSymbolUse::PointerRead
                    | InlineAsmSymbolUse::IndexedRead
                    | InlineAsmSymbolUse::IndexedReadWrite
            ) || matches!(relocation.target, NirInlineAsmTarget::InlineOffset(_))
                && matches!(
                    relocation.symbol_use,
                    InlineAsmSymbolUse::Read
                        | InlineAsmSymbolUse::ReadWrite
                        | InlineAsmSymbolUse::IndexedRead
                        | InlineAsmSymbolUse::IndexedReadWrite
                        | InlineAsmSymbolUse::PointerRead
                )
        });
        let writes_unknown = code.relocations.iter().any(|relocation| {
            matches!(
                relocation.symbol_use,
                InlineAsmSymbolUse::Call
                    | InlineAsmSymbolUse::PointerRead
                    | InlineAsmSymbolUse::IndexedWrite
                    | InlineAsmSymbolUse::IndexedReadWrite
            ) || matches!(relocation.target, NirInlineAsmTarget::InlineOffset(_))
                && matches!(
                    relocation.symbol_use,
                    InlineAsmSymbolUse::Write
                        | InlineAsmSymbolUse::ReadWrite
                        | InlineAsmSymbolUse::IndexedWrite
                        | InlineAsmSymbolUse::IndexedReadWrite
                )
        });
        let reads = inline_asm_regions(code, true);
        let writes = inline_asm_regions(code, false);
        NirMachineEffects {
            memory: NirMemoryEffects {
                reads: inline_asm_memory_access(reads, reads_unknown),
                writes: inline_asm_memory_access(writes, writes_unknown),
            },
            may_call_os: code
                .relocations
                .iter()
                .any(|relocation| relocation.symbol_use == InlineAsmSymbolUse::Call),
            opaque: false,
        }
    }

    fn nir_read_effects(&self, effects: &[SemReadEffect], opaque: bool) -> NirMemoryAccess {
        if opaque {
            return NirMemoryAccess::Unknown;
        }
        collect_memory_regions(effects.iter().map(|effect| match effect {
            SemReadEffect::Storage(storage) => self.nir_storage_region(storage),
            SemReadEffect::ZeroPage { start, end } => Some(inclusive_region(
                NirMemoryRegionKind::ZeroPage,
                u16::from(*start),
                u16::from(*end),
            )),
            SemReadEffect::Absolute { start, end } => Some(inclusive_region(
                NirMemoryRegionKind::AbsoluteRange(TargetLayout::DATA_ADDRESS_SPACE),
                *start,
                *end,
            )),
            SemReadEffect::Symbol(name) => self.nir_symbol_region(name),
            SemReadEffect::Unknown => None,
        }))
    }

    fn nir_write_effects(&self, effects: &[SemWriteEffect], opaque: bool) -> NirMemoryAccess {
        if opaque {
            return NirMemoryAccess::Unknown;
        }
        collect_memory_regions(effects.iter().map(|effect| match effect {
            SemWriteEffect::Storage(storage) => self.nir_storage_region(storage),
            SemWriteEffect::ZeroPage { start, end } => Some(inclusive_region(
                NirMemoryRegionKind::ZeroPage,
                u16::from(*start),
                u16::from(*end),
            )),
            SemWriteEffect::Absolute { start, end } => Some(inclusive_region(
                NirMemoryRegionKind::AbsoluteRange(TargetLayout::DATA_ADDRESS_SPACE),
                *start,
                *end,
            )),
            SemWriteEffect::Symbol(name) => self.nir_symbol_region(name),
            SemWriteEffect::Unknown => None,
        }))
    }

    fn nir_storage_region(&self, storage: &SemStorageRef) -> Option<NirMemoryRegion> {
        use crate::semantic::ir::SemAddressSpace;

        let size = storage.width;
        match storage.space {
            SemAddressSpace::Absolute => Some(NirMemoryRegion {
                kind: NirMemoryRegionKind::AbsoluteRange(TargetLayout::DATA_ADDRESS_SPACE),
                offset: ByteOffset::from(storage.address?.checked_add(storage.offset)?),
                size: ByteSize::from(size),
            }),
            SemAddressSpace::ZeroPage | SemAddressSpace::RuntimeZeroPage => Some(NirMemoryRegion {
                kind: NirMemoryRegionKind::ZeroPage,
                offset: ByteOffset::from(storage.address?.checked_add(storage.offset)?),
                size: ByteSize::from(size),
            }),
            SemAddressSpace::RoutineLocal => Some(NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(NirStorageId::Local(
                    self.local_id(storage.symbol.as_ref()?)?,
                )),
                offset: ByteOffset::from(storage.offset),
                size: ByteSize::from(size),
            }),
            SemAddressSpace::Parameter => Some(NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(NirStorageId::Param(
                    self.param_id(storage.symbol.as_ref()?)?,
                )),
                offset: ByteOffset::from(storage.offset),
                size: ByteSize::from(size),
            }),
            SemAddressSpace::Unknown => {
                let symbol = storage.symbol.as_ref()?;
                let mut region = self.nir_symbol_region_ref(symbol)?;
                region.offset = region
                    .offset
                    .checked_add(ByteOffset::from(storage.offset))?;
                region.size = ByteSize::from(size);
                Some(region)
            }
            SemAddressSpace::InlineStatic | SemAddressSpace::IndirectIndexedY => None,
        }
    }

    fn nir_symbol_region(&self, name: &str) -> Option<NirMemoryRegion> {
        let (id, size) = self
            .params
            .iter()
            .find(|param| param.name.eq_ignore_ascii_case(name))
            .map(|param| (NirStorageId::Param(param.id), param.ty.width))
            .or_else(|| {
                self.locals
                    .iter()
                    .find(|local| local.name.eq_ignore_ascii_case(name))
                    .map(|local| (NirStorageId::Local(local.id), local.ty.width))
            })
            .or_else(|| {
                self.global_ids
                    .iter()
                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                    .map(|(candidate, id)| {
                        (
                            NirStorageId::Global(*id),
                            self.symbol_storage_types
                                .get(candidate)
                                .and_then(|ty| ty.width),
                        )
                    })
            })?;
        Some(NirMemoryRegion {
            kind: NirMemoryRegionKind::Storage(id),
            offset: ByteOffset::ZERO,
            size: size?,
        })
    }

    fn nir_symbol_region_ref(&self, symbol: &SemSymbolRef) -> Option<NirMemoryRegion> {
        let (id, size) = self
            .param_ids_by_symbol
            .get(&symbol.id)
            .and_then(|id| {
                self.params
                    .iter()
                    .find(|param| param.id == *id)
                    .map(|param| (NirStorageId::Param(*id), param.ty.width))
            })
            .or_else(|| {
                self.local_ids_by_symbol.get(&symbol.id).and_then(|id| {
                    self.locals
                        .iter()
                        .find(|local| local.id == *id)
                        .map(|local| (NirStorageId::Local(*id), local.ty.width))
                })
            })
            .or_else(|| {
                self.global_ids_by_symbol.get(&symbol.id).map(|id| {
                    (
                        NirStorageId::Global(*id),
                        self.semantic_storage_types
                            .get(&symbol.id)
                            .and_then(|ty| ty.width),
                    )
                })
            })?;
        Some(NirMemoryRegion {
            kind: NirMemoryRegionKind::Storage(id),
            offset: ByteOffset::ZERO,
            size: size?,
        })
    }

    fn local_id(&self, symbol: &SemSymbolRef) -> Option<LocalId> {
        self.local_ids_by_symbol.get(&symbol.id).copied()
    }

    fn param_id(&self, symbol: &SemSymbolRef) -> Option<ParamId> {
        self.param_ids_by_symbol.get(&symbol.id).copied()
    }

    fn terminate(&mut self, terminator: NirTerminator) {
        if !self.current_is_open() {
            let label = format!("{}.unreachable{}", self.name, self.blocks.len());
            self.start_block(label);
        }
        self.blocks[self.current].terminator = terminator;
    }

    fn terminate_goto(&mut self, label: &str) {
        let edge = self.edge(label);
        self.terminate(NirTerminator::Goto(edge));
    }

    fn terminate_branch(&mut self, condition: NirValue, then_label: &str, else_label: &str) {
        let then_edge = self.edge(then_label);
        let else_edge = self.edge(else_label);
        self.terminate(NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        });
    }

    fn finish_open_with(&mut self, terminator: NirTerminator) {
        if self.current_is_open() {
            self.blocks[self.current].terminator = terminator;
        }
    }

    fn finish_open_goto(&mut self, label: &str) {
        let edge = self.edge(label);
        self.finish_open_with(NirTerminator::Goto(edge));
    }

    fn current_is_open(&self) -> bool {
        matches!(self.blocks[self.current].terminator, NirTerminator::Open)
    }

    fn current_label(&self) -> &str {
        &self.blocks[self.current].label
    }

    fn start_block(&mut self, label: String) {
        let id = self.block_id_for_label(&label);
        self.blocks.push(NirBlock {
            id,
            label,
            params: Vec::new(),
            ops: Vec::new(),
            terminator: NirTerminator::Open,
        });
        self.current = self.blocks.len() - 1;
    }

    fn block_id_for_label(&mut self, label: &str) -> BlockId {
        if let Some(id) = self.block_ids.get(label) {
            return *id;
        }
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.block_ids.insert(label.to_string(), id);
        id
    }

    fn edge(&mut self, label: &str) -> NirEdge {
        NirEdge {
            target: self.block_id_for_label(label),
            args: Vec::new(),
        }
    }
}

fn record_storage_sizes(program: &SemProgram) -> BTreeMap<String, u16> {
    let mut sizes = BTreeMap::new();
    for module in &program.modules {
        for item in &module.items {
            match item {
                crate::semantic::ir::SemItem::Declaration(declaration) => {
                    insert_record_storage_size(&mut sizes, declaration);
                }
                crate::semantic::ir::SemItem::Routine(routine) => {
                    for declaration in &routine.locals {
                        insert_record_storage_size(&mut sizes, declaration);
                    }
                    let mut nested = Vec::new();
                    collect_nested_declarations(&routine.body, &mut nested);
                    for declaration in nested {
                        insert_record_storage_size(&mut sizes, declaration);
                    }
                }
                _ => {}
            }
        }
    }
    sizes
}

fn insert_record_storage_size(sizes: &mut BTreeMap<String, u16>, declaration: &SemDeclaration) {
    match &declaration.storage {
        SemDeclarationStorage::Type { record_type, .. }
        | SemDeclarationStorage::Record { record_type, .. } => {
            sizes.insert(record_type.name.clone(), record_type.size);
        }
        SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. } => {}
    }
}

#[derive(Default)]
struct MachineDefines {
    ids: BTreeMap<usize, Vec<MachineItem>>,
    names: BTreeMap<String, Vec<MachineItem>>,
}

fn collect_machine_defines(program: &SemProgram) -> MachineDefines {
    let mut defines = MachineDefines::default();
    for module in &program.modules {
        for item in &module.items {
            collect_machine_defines_from_item(item, &mut defines);
        }
    }
    defines
}

fn collect_machine_defines_from_item(
    item: &crate::semantic::ir::SemItem,
    defines: &mut MachineDefines,
) {
    match item {
        crate::semantic::ir::SemItem::Define(define) => {
            if let Some(items) = parse_machine_define_value(&define.value) {
                insert_machine_define(defines, define.symbol.id.0, &define.symbol.name, items);
            }
        }
        crate::semantic::ir::SemItem::Routine(routine) => {
            collect_machine_define_ids_from_statements(&routine.body, &mut defines.ids);
        }
        crate::semantic::ir::SemItem::Statement(stmt) => {
            collect_machine_defines_from_stmt(stmt, defines);
        }
        crate::semantic::ir::SemItem::Include(_)
        | crate::semantic::ir::SemItem::Const(_)
        | crate::semantic::ir::SemItem::Set(_)
        | crate::semantic::ir::SemItem::Declaration(_)
        | crate::semantic::ir::SemItem::Unsupported { .. } => {}
    }
}

fn collect_machine_defines_from_statements(statements: &[SemStmt], defines: &mut MachineDefines) {
    for stmt in statements {
        collect_machine_defines_from_stmt(stmt, defines);
    }
}

fn collect_machine_define_ids_from_statements(
    statements: &[SemStmt],
    ids: &mut BTreeMap<usize, Vec<MachineItem>>,
) {
    for stmt in statements {
        collect_machine_define_ids_from_stmt(stmt, ids);
    }
}

fn collect_machine_define_ids_from_stmt(
    stmt: &SemStmt,
    ids: &mut BTreeMap<usize, Vec<MachineItem>>,
) {
    match stmt {
        SemStmt::LexicalBlock { body, .. } => {
            collect_machine_define_ids_from_statements(body, ids);
        }
        SemStmt::Define(define) => {
            if let Some(items) = parse_machine_define_value(&define.value) {
                ids.insert(define.symbol.id.0, items);
            }
        }
        SemStmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                collect_machine_define_ids_from_statements(&branch.body, ids);
            }
            collect_machine_define_ids_from_statements(else_body, ids);
        }
        SemStmt::While { body, .. } | SemStmt::DoUntil { body, .. } | SemStmt::For { body, .. } => {
            collect_machine_define_ids_from_statements(body, ids);
        }
        SemStmt::Return { .. }
        | SemStmt::Exit { .. }
        | SemStmt::Assign { .. }
        | SemStmt::RecordCopy { .. }
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
        | SemStmt::InlineAsm { .. }
        | SemStmt::Unsupported { .. } => {}
    }
}

fn machine_define_names_from_statements(
    statements: &[SemStmt],
) -> BTreeMap<String, Vec<MachineItem>> {
    let mut names = BTreeMap::new();
    collect_machine_define_names_from_statements(statements, &mut names);
    names
}

fn collect_machine_define_names_from_statements(
    statements: &[SemStmt],
    names: &mut BTreeMap<String, Vec<MachineItem>>,
) {
    for stmt in statements {
        collect_machine_define_names_from_stmt(stmt, names);
    }
}

fn collect_machine_define_names_from_stmt(
    stmt: &SemStmt,
    names: &mut BTreeMap<String, Vec<MachineItem>>,
) {
    match stmt {
        SemStmt::LexicalBlock { body, .. } => {
            collect_machine_define_names_from_statements(body, names);
        }
        SemStmt::Define(define) => {
            if let Some(items) = parse_machine_define_value(&define.value) {
                names.insert(storage_key(&define.symbol.name), items);
            }
        }
        SemStmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                collect_machine_define_names_from_statements(&branch.body, names);
            }
            collect_machine_define_names_from_statements(else_body, names);
        }
        SemStmt::While { body, .. } | SemStmt::DoUntil { body, .. } | SemStmt::For { body, .. } => {
            collect_machine_define_names_from_statements(body, names);
        }
        SemStmt::Return { .. }
        | SemStmt::Exit { .. }
        | SemStmt::Assign { .. }
        | SemStmt::RecordCopy { .. }
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
        | SemStmt::InlineAsm { .. }
        | SemStmt::Unsupported { .. } => {}
    }
}

fn collect_machine_defines_from_stmt(stmt: &SemStmt, defines: &mut MachineDefines) {
    match stmt {
        SemStmt::LexicalBlock { body, .. } => {
            collect_machine_defines_from_statements(body, defines);
        }
        SemStmt::Define(define) => {
            if let Some(items) = parse_machine_define_value(&define.value) {
                insert_machine_define(defines, define.symbol.id.0, &define.symbol.name, items);
            }
        }
        SemStmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                collect_machine_defines_from_statements(&branch.body, defines);
            }
            collect_machine_defines_from_statements(else_body, defines);
        }
        SemStmt::While { body, .. } | SemStmt::DoUntil { body, .. } | SemStmt::For { body, .. } => {
            collect_machine_defines_from_statements(body, defines);
        }
        SemStmt::Return { .. }
        | SemStmt::Exit { .. }
        | SemStmt::Assign { .. }
        | SemStmt::RecordCopy { .. }
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
        | SemStmt::InlineAsm { .. }
        | SemStmt::Unsupported { .. } => {}
    }
}

fn insert_machine_define(
    defines: &mut MachineDefines,
    id: usize,
    name: &str,
    items: Vec<MachineItem>,
) {
    defines.ids.insert(id, items.clone());
    defines.names.insert(storage_key(name), items);
}

fn parse_machine_define_value(value: &str) -> Option<Vec<MachineItem>> {
    let tokens = tokenize(value).ok()?;
    if matches!(tokens.first()?.kind, TokenKind::LBracket) {
        return crate::parser::parse_machine_items(&tokens).ok();
    }

    let mut tokens = tokens
        .into_iter()
        .filter(|token| token.kind != TokenKind::Eof);
    let token = tokens.next()?;
    if tokens.next().is_some() {
        return None;
    }
    let item = match token.kind {
        TokenKind::Number(number) => MachineItem::Number(number),
        TokenKind::String(value) => MachineItem::StringLiteral(value),
        TokenKind::Char(value) => MachineItem::CharLiteral(value),
        _ => return None,
    };
    Some(vec![item])
}

fn collect_temps(blocks: &[NirBlock]) -> Vec<NirTemp> {
    let mut temps = Vec::new();
    for block in blocks {
        temps.extend(block.params.iter().map(|param| NirTemp {
            id: param.dest,
            ty: param.ty.clone(),
            def: NirTempDef {
                block: block.id,
                op_index: None,
            },
        }));
        for (op_index, op) in block.ops.iter().enumerate() {
            if let Some((id, ty)) = op_temp_def(op) {
                temps.push(NirTemp {
                    id,
                    ty: ty.clone(),
                    def: NirTempDef {
                        block: block.id,
                        op_index: Some(op_index),
                    },
                });
            }
        }
    }
    temps
}

fn op_temp_def(op: &NirOp) -> Option<(TempId, &NirType)> {
    match op {
        NirOp::Load { dest, ty, .. }
        | NirOp::VolatileLoad { dest, ty, .. }
        | NirOp::AddrOf { dest, ty, .. }
        | NirOp::Unary { dest, ty, .. }
        | NirOp::PointerOffset { dest, ty, .. }
        | NirOp::Binary { dest, ty, .. }
        | NirOp::Compare { dest, ty, .. } => Some((*dest, ty)),
        NirOp::Real(NirRealOp::Compare {
            result,
            result_type,
            ..
        })
        | NirOp::Real(NirRealOp::RealToInteger {
            result,
            result_type,
            ..
        }) => Some((*result, result_type)),
        NirOp::Cast { dest, to, .. } => Some((*dest, to)),
        NirOp::Call {
            result: Some(result),
            ..
        } => Some((result.dest, &result.ty)),
        NirOp::RuntimeHelperOverride { .. }
        | NirOp::Store { .. }
        | NirOp::VolatileStore { .. }
        | NirOp::CopyBytes { .. }
        | NirOp::Real(_)
        | NirOp::Call { result: None, .. }
        | NirOp::MachineBlock { .. }
        | NirOp::InlineAsm { .. }
        | NirOp::Unsupported { .. } => None,
    }
}

fn declaration_kind(declaration: &SemDeclaration) -> String {
    let mut kind = match &declaration.storage {
        SemDeclarationStorage::Scalar => type_summary(&declaration.ty.value),
        SemDeclarationStorage::Array {
            array_type,
            length,
            action_storage,
            origin,
            fixed_address: _,
        } => format!(
            "array {:?} length={} storage={action_storage:?} origin={}",
            array_type,
            length
                .as_ref()
                .map(expr_summary)
                .unwrap_or_else(|| "?".to_string()),
            array_origin_summary(*origin)
        ),
        SemDeclarationStorage::Type {
            record_type,
            fields,
        } => {
            format!("type {record_type:?} fields={}", fields.len())
        }
        SemDeclarationStorage::Record {
            record_type,
            fields,
        } => {
            format!("record {record_type:?} fields={}", fields.len())
        }
    };
    if let Some(symbol) = routine_symbol_initializer(declaration) {
        kind.push_str(&format!(" pointer_init={symbol}"));
    }
    kind
}

fn declaration_storage_class(storage: &SemDeclarationStorage) -> NirStorageClass {
    match storage {
        SemDeclarationStorage::Scalar => NirStorageClass::Scalar,
        SemDeclarationStorage::Array { .. } => NirStorageClass::Array,
        SemDeclarationStorage::Record { .. } => NirStorageClass::Record,
        SemDeclarationStorage::Type { .. } => NirStorageClass::Type,
    }
}

fn routine_symbol_initializer(declaration: &SemDeclaration) -> Option<&str> {
    let initializer = declaration.initializer.as_ref()?;
    let SemExprKind::Symbol(symbol) = &initializer.kind else {
        return None;
    };
    if matches!(
        symbol.class,
        crate::semantic::SymbolClass::Proc | crate::semantic::SymbolClass::Func
    ) {
        Some(symbol.name.as_str())
    } else {
        None
    }
}

fn declaration_storage_size(
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
    address_initializer: Option<u16>,
    target_layout: TargetLayout,
) -> u16 {
    match &declaration.storage {
        SemDeclarationStorage::Scalar => declaration
            .ty
            .value
            .value_width_bytes_for_layout(target_layout)
            .or_else(|| {
                declaration
                    .ty
                    .value
                    .as_record_name()
                    .and_then(|name| record_storage_sizes.get(name).copied())
            })
            .unwrap_or(0),
        SemDeclarationStorage::Array { array_type, .. } => declaration_array_storage_size(
            declaration,
            array_type,
            record_storage_sizes,
            address_initializer,
            target_layout,
        ),
        SemDeclarationStorage::Type { .. } => 0,
        SemDeclarationStorage::Record { record_type, .. } => record_type.size,
    }
}

fn declaration_is_array(declaration: &SemDeclaration) -> bool {
    matches!(declaration.storage, SemDeclarationStorage::Array { .. })
}

fn declaration_has_storage(declaration: &SemDeclaration) -> bool {
    matches!(
        declaration.storage,
        SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. }
    )
}

fn semantic_local_display_name(symbol: &SemSymbolRef) -> String {
    symbol
        .lexical_display_name
        .clone()
        .unwrap_or_else(|| symbol.name.clone())
}

fn storage_alias_initializer_expr(expr: &SemExpr) -> Option<(&SemSymbolRef, u16)> {
    match &expr.kind {
        SemExprKind::Symbol(symbol) => Some((symbol, 0)),
        SemExprKind::LValue(lvalue) => lvalue_symbol(lvalue).map(|symbol| (symbol, 0)),
        SemExprKind::ArrayDecay(decay) => lvalue_symbol(&decay.array).map(|symbol| (symbol, 0)),
        SemExprKind::Cast { expr, .. } => storage_alias_initializer_expr(expr),
        SemExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } => {
            let (symbol, base_offset) = storage_alias_initializer_expr(left)?;
            let offset = literal_expr_u16(right)?;
            Some((symbol, base_offset.wrapping_add(offset)))
        }
        _ => None,
    }
}

fn literal_expr_u16(expr: &SemExpr) -> Option<u16> {
    match &expr.kind {
        SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
        SemExprKind::Literal(SemLiteral::Constant(value)) => Some(value.bits),
        SemExprKind::Cast { expr, .. } => literal_expr_u16(expr),
        _ => None,
    }
}

fn declaration_array_storage_size(
    declaration: &SemDeclaration,
    array_type: &ArrayType,
    record_storage_sizes: &BTreeMap<String, u16>,
    address_initializer: Option<u16>,
    target_layout: TargetLayout,
) -> u16 {
    let elem_size =
        array_element_width(array_type, record_storage_sizes, target_layout).unwrap_or(1);
    let initializer_byte_len = array_initializer_byte_len(declaration, elem_size);
    if array_type.length.is_none() && initializer_byte_len.is_some() {
        return target_layout.data_pointer.size_bytes.get() as u16;
    }
    if address_initializer.is_some()
        && declaration_array_address_initializer_uses_pointer_storage(
            declaration,
            record_storage_sizes,
            target_layout,
        )
    {
        return array_descriptor_size(target_layout, array_type.length.is_some()).get() as u16;
    }
    if elem_size > 1 && initializer_byte_len.is_some() {
        return array_descriptor_size(target_layout, array_type.length.is_some()).get() as u16;
    }
    if elem_size == 1
        && let Some(byte_len) = string_initializer_bytes(declaration)
            .map(|bytes| bytes.len())
            .or(initializer_byte_len)
    {
        return array_type
            .length
            .map(|length| length.saturating_mul(elem_size))
            .unwrap_or(byte_len as u16)
            .max(byte_len as u16);
    }
    array_type
        .length
        .map(|length| length.saturating_mul(elem_size))
        .unwrap_or(target_layout.data_pointer.size_bytes.get() as u16)
}

fn declaration_array_address_initializer_uses_pointer_storage(
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
    target_layout: TargetLayout,
) -> bool {
    let SemDeclarationStorage::Array { array_type, .. } = &declaration.storage else {
        return false;
    };
    if declaration.initializer.is_none() {
        return false;
    }
    let elem_size =
        array_element_width(array_type, record_storage_sizes, target_layout).unwrap_or(1);
    match array_type.length {
        None => true,
        Some(length) => length.saturating_mul(elem_size) > 0x0100,
    }
}

fn symbolic_array_initializer_routine(declaration: &SemDeclaration) -> Option<String> {
    let initializer = declaration.initializer.as_ref()?;
    symbolic_array_initializer_routine_expr(initializer)
}

fn symbolic_array_initializer_routine_expr(expr: &SemExpr) -> Option<String> {
    match &expr.kind {
        SemExprKind::Cast { expr, .. } => symbolic_array_initializer_routine_expr(expr),
        SemExprKind::Symbol(symbol)
            if matches!(
                symbol.class,
                SymbolClass::Proc
                    | SymbolClass::Func
                    | SymbolClass::BuiltinProc
                    | SymbolClass::BuiltinFunc
            ) =>
        {
            Some(symbol.name.clone())
        }
        _ => None,
    }
}

fn array_element_width(
    array_type: &ArrayType,
    record_storage_sizes: &BTreeMap<String, u16>,
    target_layout: TargetLayout,
) -> Option<u16> {
    array_type
        .element
        .value_width_bytes_for_layout(target_layout)
        .or_else(|| {
        array_type
            .element
            .as_record_name()
            .and_then(|name| record_storage_sizes.get(name).copied())
        })
}

fn array_descriptor_size(target_layout: TargetLayout, has_size_word: bool) -> ByteSize {
    target_layout.data_pointer.size_bytes.saturating_add(if has_size_word {
        ByteSize::new(2)
    } else {
        ByteSize::ZERO
    })
}

fn callable_descriptor_size(target_layout: TargetLayout, has_size_word: bool) -> ByteSize {
    target_layout.code_pointer.size_bytes.saturating_add(if has_size_word {
        ByteSize::new(2)
    } else {
        ByteSize::ZERO
    })
}

fn declaration_array_fact(
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
    address_initializer: Option<u16>,
    target_layout: TargetLayout,
) -> Option<NirArrayGlobalFact> {
    let SemDeclarationStorage::Array { array_type, .. } = &declaration.storage else {
        return None;
    };
    let elem_size =
        array_element_width(array_type, record_storage_sizes, target_layout).unwrap_or(1);
    let initializer_is_data_image = matches!(
        declaration.initializer.as_ref().map(|expr| &expr.kind),
        Some(SemExprKind::InitializerList(_))
    );
    Some(NirArrayGlobalFact {
        elem_size: ByteSize::from(elem_size),
        length: array_type.length,
        pointer_backed: (array_type.length.is_none() && declaration.initializer.is_none())
            || (address_initializer.is_some()
                && declaration_array_address_initializer_uses_pointer_storage(
                    declaration,
                    record_storage_sizes,
                    target_layout,
                ))
            || symbolic_array_initializer_routine(declaration).is_some()
            || (initializer_is_data_image && (elem_size > 1 || array_type.length.is_none())),
        address_initializer: address_initializer
            .map(|address| AddressValue::data(u64::from(address))),
    })
}

fn declaration_symbol_storage_type(
    declaration: &SemDeclaration,
    _address_initializer: Option<u16>,
) -> Option<NirType> {
    let SemDeclarationStorage::Array { array_type, .. } = &declaration.storage else {
        return None;
    };
    if array_type.length.is_none() && declaration.initializer.is_none() {
        return Some(NirFacts::type_from_value(&array_type.pointer_type()));
    }
    None
}

fn declaration_global_init(
    id: SymbolId,
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
    backing: &NirGlobalBacking,
    address_initializer: Option<u16>,
    global_ids: &BTreeMap<String, SymbolId>,
    routine_ids: &BTreeMap<String, u32>,
    target_layout: TargetLayout,
) -> Option<NirGlobalInit> {
    if matches!(backing, NirGlobalBacking::Absolute(_)) {
        return None;
    }
    let storage_size =
        declaration_storage_size(
            declaration,
            record_storage_sizes,
            address_initializer,
            target_layout,
        );
    match &declaration.storage {
        SemDeclarationStorage::Scalar => match &declaration.static_initializer {
            Some(initializer) => Some(data_image_init(
                static_initializer_data_image(initializer, |target| {
                    global_data_relocation_target(target, global_ids, routine_ids)
                })
                .expect("verified semantic static initializer must lower to a NIR data image"),
                storage_size,
            )),
            None => scalar_initializer_bytes(declaration, storage_size)
                .map(|bytes| bytes_init(bytes, storage_size)),
        },
        SemDeclarationStorage::Array { array_type, .. } => {
            let elem_size =
                array_element_width(array_type, record_storage_sizes, target_layout).unwrap_or(1);
            let data_image = match &declaration.static_initializer {
                Some(initializer) => Some(
                    static_initializer_data_image(initializer, |target| {
                        global_data_relocation_target(target, global_ids, routine_ids)
                    })
                    .expect("verified semantic static initializer must lower to a NIR data image"),
                ),
                None => declaration.initializer.as_ref().and_then(|initializer| {
                    legacy_scalar_array_initializer_data_image(
                        initializer,
                        elem_size,
                        array_type.element.is_real(),
                        |target| global_data_relocation_target(target, global_ids, routine_ids),
                    )
                }),
            };
            if let Some(address) = address_initializer
                && declaration_array_address_initializer_uses_pointer_storage(
                    declaration,
                    record_storage_sizes,
                    target_layout,
                )
            {
                let address = address.to_le_bytes();
                let bytes = if array_type.length.is_some() {
                    vec![address[0], address[1], address[0], address[1]]
                } else {
                    vec![address[0], address[1]]
                };
                return Some(bytes_init(bytes, storage_size));
            }
            if let Some(name) = symbolic_array_initializer_routine(declaration) {
                return Some(NirGlobalInit::RoutineAddress {
                    routine: *routine_ids
                        .get(&storage_key(&name))
                        .expect("resolved routine initializer must have a routine id"),
                    descriptor_size: callable_descriptor_size(
                        target_layout,
                        array_type.length.is_some(),
                    ),
                    size_word: None,
                    mutable: true,
                    section: "global".to_string(),
                });
            }
            if elem_size > 1
                && let Some(image) = data_image.clone()
            {
                let len = array_type
                    .length
                    .unwrap_or((image.bytes.len() as u16) / elem_size);
                let byte_size = elem_size.saturating_mul(len).max(image.bytes.len() as u16);
                return Some(NirGlobalInit::Descriptor {
                    backing: NirDataBacking {
                        owner: id,
                        zero_fill: ByteSize::from(
                            byte_size.saturating_sub(image.bytes.len() as u16),
                        ),
                        image,
                        section: "global.backing".to_string(),
                    },
                    descriptor_size: array_descriptor_size(
                        target_layout,
                        array_type.length.is_some(),
                    ),
                    size_word: array_type.length.map(|_| 0),
                    mutable: true,
                    section: "global".to_string(),
                });
            }
            if array_type.length.is_none()
                && elem_size == 1
                && let Some(image) = data_image.clone()
            {
                return Some(NirGlobalInit::Descriptor {
                    backing: NirDataBacking {
                        owner: id,
                        zero_fill: ByteSize::ZERO,
                        image,
                        section: "global.backing".to_string(),
                    },
                    descriptor_size: array_descriptor_size(target_layout, false),
                    size_word: None,
                    mutable: true,
                    section: "global".to_string(),
                });
            }
            let bytes = if elem_size == 1 {
                string_initializer_bytes(declaration)
            } else {
                None
            };
            let image = data_image.or_else(|| bytes.map(NirDataImage::literal));
            if let Some(image) = image {
                let total_size = array_type
                    .length
                    .map(|length| length.saturating_mul(elem_size))
                    .unwrap_or(image.bytes.len() as u16)
                    .max(image.bytes.len() as u16);
                return Some(data_image_init(image, total_size));
            }
            array_type.length.map(|length| {
                let bytes = length.saturating_mul(elem_size);
                NirGlobalInit::ZeroFill {
                    bytes: ByteSize::from(bytes),
                    mutable: true,
                    section: "global".to_string(),
                }
            })
        }
        SemDeclarationStorage::Record { .. } => None,
        SemDeclarationStorage::Type { .. } => None,
    }
}

fn bytes_init(bytes: Vec<u8>, total_size: u16) -> NirGlobalInit {
    data_image_init(NirDataImage::literal(bytes), total_size)
}

fn data_image_init(image: NirDataImage, total_size: u16) -> NirGlobalInit {
    let zero_fill = total_size.saturating_sub(image.bytes.len() as u16);
    NirGlobalInit::Bytes {
        image,
        zero_fill: ByteSize::from(zero_fill),
        mutable: true,
        section: "global".to_string(),
    }
}

fn apply_program_end_symbol_set(globals: &mut [NirGlobal], set: &SemSet) -> bool {
    let symbol = match &set.address.kind {
        SemExprKind::Symbol(symbol) => symbol,
        SemExprKind::LValue(lvalue) => match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => symbol,
            _ => return false,
        },
        SemExprKind::ArrayDecay(decay) => match &decay.array.kind {
            SemLValueKind::Symbol(symbol) => symbol,
            _ => return false,
        },
        _ => return false,
    };
    if !matches!(set.value.kind, SemExprKind::CurrentLocation) {
        return false;
    }
    let Some(global) = globals
        .iter_mut()
        .find(|global| storage_key(&global.name) == storage_key(&symbol.name))
    else {
        return false;
    };
    if matches!(global.backing, NirGlobalBacking::Absolute(_))
        || global.storage_size.get() < 2
    {
        return false;
    }
    global.init = Some(NirGlobalInit::ProgramEndWord {
        mutable: true,
        section: "global".to_string(),
    });
    true
}

fn declaration_local_init(
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
    backing: &NirLocalBacking,
    global_ids: &BTreeMap<String, SymbolId>,
    routine_ids: &BTreeMap<String, u32>,
    param_ids: &BTreeMap<SemSymbolId, ParamId>,
    local_ids: &BTreeMap<SemSymbolId, LocalId>,
    target_layout: TargetLayout,
) -> Option<NirStorageInit> {
    if matches!(
        backing,
        NirLocalBacking::Absolute(_)
            | NirLocalBacking::Alias { .. }
            | NirLocalBacking::GlobalAlias { .. }
    ) {
        return None;
    }
    let storage_size =
        declaration_storage_size(declaration, record_storage_sizes, None, target_layout);
    match &declaration.storage {
        SemDeclarationStorage::Scalar => {
            if let Some(initializer) = &declaration.static_initializer {
                let image = static_initializer_data_image(initializer, |target| {
                    local_data_relocation_target(
                        target,
                        global_ids,
                        routine_ids,
                        param_ids,
                        local_ids,
                    )
                })
                .expect("verified semantic static initializer must lower to a NIR data image");
                return Some(storage_data_image_init(image, storage_size));
            }
            if let Some(bytes) = scalar_initializer_bytes(declaration, storage_size) {
                return Some(storage_bytes_init(bytes, storage_size));
            }
            if storage_size > declaration.ty.value.value_width_bytes().unwrap_or(0) {
                return Some(NirStorageInit::ZeroFill {
                    bytes: ByteSize::from(storage_size),
                    mutable: true,
                    section: "local".to_string(),
                });
            }
            None
        }
        SemDeclarationStorage::Array { array_type, .. } => {
            let elem_size =
                array_element_width(array_type, record_storage_sizes, target_layout).unwrap_or(1);
            let data_image = match &declaration.static_initializer {
                Some(initializer) => Some(
                    static_initializer_data_image(initializer, |target| {
                        local_data_relocation_target(
                            target,
                            global_ids,
                            routine_ids,
                            param_ids,
                            local_ids,
                        )
                    })
                    .expect("verified semantic static initializer must lower to a NIR data image"),
                ),
                None => declaration.initializer.as_ref().and_then(|initializer| {
                    legacy_scalar_array_initializer_data_image(
                        initializer,
                        elem_size,
                        array_type.element.is_real(),
                        |target| {
                            local_data_relocation_target(
                                target,
                                global_ids,
                                routine_ids,
                                param_ids,
                                local_ids,
                            )
                        },
                    )
                }),
            };
            if elem_size > 1
                && let Some(image) = data_image.clone()
            {
                let len = array_type
                    .length
                    .unwrap_or((image.bytes.len() as u16) / elem_size);
                let byte_size = elem_size.saturating_mul(len).max(image.bytes.len() as u16);
                return Some(NirStorageInit::Descriptor {
                    backing: NirStorageBacking {
                        zero_fill: ByteSize::from(
                            byte_size.saturating_sub(image.bytes.len() as u16),
                        ),
                        image,
                        section: "local.backing".to_string(),
                    },
                    descriptor_size: array_descriptor_size(
                        target_layout,
                        array_type.length.is_some(),
                    ),
                    size_word: array_type.length.map(|_| 0),
                    mutable: true,
                    section: "local".to_string(),
                });
            }
            let bytes = if elem_size == 1 {
                string_initializer_bytes(declaration)
            } else {
                None
            };
            let image = data_image.or_else(|| bytes.map(NirDataImage::literal));
            if let Some(image) = image {
                let total_size = array_type
                    .length
                    .map(|length| length.saturating_mul(elem_size))
                    .unwrap_or(image.bytes.len() as u16)
                    .max(image.bytes.len() as u16);
                return Some(storage_data_image_init(image, total_size));
            }
            array_type.length.map(|length| {
                let bytes = length.saturating_mul(elem_size);
                NirStorageInit::ZeroFill {
                    bytes: ByteSize::from(bytes),
                    mutable: true,
                    section: "local".to_string(),
                }
            })
        }
        SemDeclarationStorage::Record { .. } | SemDeclarationStorage::Type { .. } => None,
    }
}

fn storage_bytes_init(bytes: Vec<u8>, total_size: u16) -> NirStorageInit {
    storage_data_image_init(NirDataImage::literal(bytes), total_size)
}

fn storage_data_image_init(image: NirDataImage, total_size: u16) -> NirStorageInit {
    let zero_fill = total_size.saturating_sub(image.bytes.len() as u16);
    NirStorageInit::Bytes {
        image,
        zero_fill: ByteSize::from(zero_fill),
        mutable: true,
        section: "local".to_string(),
    }
}

fn string_literal_storage_bytes(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes.push(
        u8::try_from(text.chars().count())
            .map_err(|_| "string literal is too long for an ACTION! length prefix".to_string())?,
    );
    for ch in text.chars() {
        bytes.push(
            source_char_byte(ch)
                .ok_or_else(|| format!("character `{ch}` is outside byte source encoding"))?,
        );
    }
    Ok(bytes)
}

fn scalar_initializer_bytes(declaration: &SemDeclaration, total_size: u16) -> Option<Vec<u8>> {
    if declaration.ty.value.is_real() {
        return match &declaration.initializer.as_ref()?.kind {
            SemExprKind::Literal(SemLiteral::Real { value, .. }) => Some(value.to_bytes().to_vec()),
            SemExprKind::InitializerList(elements) if elements.len() == 1 => {
                sem_initializer_real_value(&elements[0]).map(|value| value.to_bytes().to_vec())
            }
            _ => None,
        };
    }
    let value = literal_number_u16_expr(declaration.initializer.as_ref()?).or_else(|| {
        let values = numeric_initializer_values(declaration.initializer.as_ref()?)?;
        (values.len() == 1).then_some(values[0])
    })?;
    let mut bytes = Vec::with_capacity(usize::from(total_size.min(2)));
    if total_size > 0 {
        bytes.push(value as u8);
    }
    if total_size > 1 {
        bytes.push((value >> 8) as u8);
    }
    Some(bytes)
}

fn literal_number_u16_expr(expr: &SemExpr) -> Option<u16> {
    match &expr.kind {
        SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
        _ => None,
    }
}

fn string_initializer_bytes(declaration: &SemDeclaration) -> Option<Vec<u8>> {
    let SemExprKind::Literal(SemLiteral::String(value)) = &declaration.initializer.as_ref()?.kind
    else {
        return None;
    };
    let literal_bytes = value
        .chars()
        .map(|ch| if ch.is_ascii() { ch as u8 } else { b'?' })
        .collect::<Vec<_>>();
    let mut bytes = Vec::with_capacity(literal_bytes.len().saturating_add(1));
    bytes.push(literal_bytes.len() as u8);
    bytes.extend(literal_bytes);
    Some(bytes)
}

fn numeric_initializer_bytes(declaration: &SemDeclaration, elem_size: u16) -> Option<Vec<u8>> {
    let values = numeric_initializer_values(declaration.initializer.as_ref()?)?;
    let mut bytes = Vec::with_capacity(values.len().saturating_mul(usize::from(elem_size)));
    for value in values {
        bytes.push(value as u8);
        if elem_size == 2 {
            bytes.push((value >> 8) as u8);
        } else if elem_size != 1 {
            return None;
        }
    }
    Some(bytes)
}

// Compatibility adapter for scalar arrays that predate SemIR static plans.
// Aggregate declarations must use `static_initializer_data_image` instead.
fn legacy_scalar_array_initializer_data_image(
    expr: &SemExpr,
    elem_size: u16,
    real_elements: bool,
    mut resolve_target: impl FnMut(&SemSymbolRef) -> Option<NirDataRelocationTarget>,
) -> Option<NirDataImage> {
    if !matches!(elem_size, 1 | 2) && !(real_elements && elem_size == 6) {
        return None;
    }
    let SemExprKind::InitializerList(elements) = &expr.kind else {
        return None;
    };
    let mut image = NirDataImage::default();
    for element in elements {
        match &element.kind {
            SemInitializerElementKind::Literal { .. } => {
                if real_elements {
                    image.bytes.extend(
                        sem_initializer_real_value(element)
                            .expect("verified REAL initializer literal")
                            .to_bytes(),
                    );
                    continue;
                }
                let value = sem_initializer_literal_value(element)
                    .expect("verified SemIR initializer literal must have a constant value");
                image.bytes.push(value as u8);
                if elem_size == 2 {
                    image.bytes.push((value >> 8) as u8);
                }
            }
            SemInitializerElementKind::Address {
                selector,
                target,
                addend,
            } => {
                if real_elements {
                    return None;
                }
                let kind = match selector {
                    Some(AddressByteSelector::Low) => NirDataRelocationKind::Low8,
                    Some(AddressByteSelector::High) => NirDataRelocationKind::High8,
                    None => NirDataRelocationKind::Word16,
                };
                let width = kind.width();
                debug_assert_eq!(width.get(), u32::from(elem_size));
                let offset = ByteOffset::try_from(image.bytes.len())
                    .expect("verified static initializer must fit in NIR storage");
                image
                    .bytes
                    .resize(image.bytes.len().saturating_add(usize::from(width)), 0);
                image.relocations.push(NirDataRelocation {
                    offset,
                    kind,
                    target: resolve_target(target)
                        .expect("verified SemIR initializer target must have a NIR identity"),
                    addend: *addend,
                    span: element.span,
                });
            }
            SemInitializerElementKind::Invalid => {
                unreachable!("invalid initializer elements must be rejected before NIR lowering")
            }
        }
    }
    Some(image)
}

fn static_initializer_data_image(
    initializer: &SemStaticInitializer,
    mut resolve_target: impl FnMut(&SemSymbolRef) -> Option<NirDataRelocationTarget>,
) -> Option<NirDataImage> {
    let mut image = NirDataImage::literal(vec![0; usize::from(initializer.initialized_extent)]);
    for write in &initializer.writes {
        let offset = usize::from(write.offset);
        let end = offset.checked_add(usize::from(write.width))?;
        let destination = image.bytes.get_mut(offset..end)?;
        match &write.value {
            SemStaticInitializerValue::Literal { .. } if write.destination.is_real() => {
                let bytes = sem_static_initializer_real_value(&write.value)?.to_bytes();
                if destination.len() != bytes.len() {
                    return None;
                }
                destination.copy_from_slice(&bytes);
            }
            SemStaticInitializerValue::Literal { .. } => {
                let value = sem_static_initializer_literal_value(&write.value)?;
                match destination {
                    [low] => *low = value as u8,
                    [low, high] => {
                        *low = value as u8;
                        *high = (value >> 8) as u8;
                    }
                    _ => return None,
                }
            }
            SemStaticInitializerValue::Address {
                selector,
                target,
                addend,
            } => {
                let kind = match selector {
                    Some(AddressByteSelector::Low) => NirDataRelocationKind::Low8,
                    Some(AddressByteSelector::High) => NirDataRelocationKind::High8,
                    None => NirDataRelocationKind::Word16,
                };
                if kind.width().get() != u32::from(write.width) {
                    return None;
                }
                image.relocations.push(NirDataRelocation {
                    offset: ByteOffset::from(write.offset),
                    kind,
                    target: resolve_target(target)?,
                    addend: *addend,
                    span: write.span,
                });
            }
        }
    }
    Some(image)
}

fn array_initializer_byte_len(declaration: &SemDeclaration, elem_size: u16) -> Option<usize> {
    if let Some(initializer) = &declaration.static_initializer {
        return Some(usize::from(initializer.initialized_extent));
    }
    let real_elements = matches!(
        &declaration.storage,
        SemDeclarationStorage::Array { array_type, .. } if array_type.element.is_real()
    );
    match &declaration.initializer.as_ref()?.kind {
        SemExprKind::InitializerList(elements)
            if matches!(elem_size, 1 | 2) || (real_elements && elem_size == 6) =>
        {
            Some(elements.len().saturating_mul(usize::from(elem_size)))
        }
        _ => numeric_initializer_bytes(declaration, elem_size).map(|bytes| bytes.len()),
    }
}

fn global_data_relocation_target(
    target: &SemSymbolRef,
    global_ids: &BTreeMap<String, SymbolId>,
    routine_ids: &BTreeMap<String, u32>,
) -> Option<NirDataRelocationTarget> {
    if matches!(
        target.class,
        SymbolClass::Proc | SymbolClass::Func | SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc
    ) {
        return routine_ids
            .get(&storage_key(&target.name))
            .copied()
            .map(NirDataRelocationTarget::Routine);
    }
    global_ids
        .iter()
        .find(|(name, _)| storage_key(name) == storage_key(&target.name))
        .map(|(_, id)| NirDataRelocationTarget::Storage(NirStorageId::Global(*id)))
        .or_else(|| {
            resident_variable(&target.name)
                .map(|variable| {
                    NirDataRelocationTarget::Absolute(AddressValue::data(u64::from(
                        variable.address,
                    )))
                })
        })
}

fn local_data_relocation_target(
    target: &SemSymbolRef,
    global_ids: &BTreeMap<String, SymbolId>,
    routine_ids: &BTreeMap<String, u32>,
    param_ids: &BTreeMap<SemSymbolId, ParamId>,
    local_ids: &BTreeMap<SemSymbolId, LocalId>,
) -> Option<NirDataRelocationTarget> {
    local_ids
        .get(&target.id)
        .copied()
        .map(|id| NirDataRelocationTarget::Storage(NirStorageId::Local(id)))
        .or_else(|| {
            param_ids
                .get(&target.id)
                .copied()
                .map(|id| NirDataRelocationTarget::Storage(NirStorageId::Param(id)))
        })
        .or_else(|| global_data_relocation_target(target, global_ids, routine_ids))
}

fn increment_data_image_routine_ids_in_global_init(init: &mut NirGlobalInit) {
    match init {
        NirGlobalInit::Bytes { image, .. } => increment_data_image_routine_ids(image),
        NirGlobalInit::Descriptor { backing, .. } => {
            increment_data_image_routine_ids(&mut backing.image)
        }
        NirGlobalInit::RoutineAddress { routine, .. } => {
            *routine = routine.saturating_add(1);
        }
        NirGlobalInit::ZeroFill { .. } | NirGlobalInit::ProgramEndWord { .. } => {}
    }
}

fn increment_data_image_routine_ids_in_storage_init(init: &mut NirStorageInit) {
    match init {
        NirStorageInit::Bytes { image, .. } => increment_data_image_routine_ids(image),
        NirStorageInit::Descriptor { backing, .. } => {
            increment_data_image_routine_ids(&mut backing.image)
        }
        NirStorageInit::ZeroFill { .. } => {}
    }
}

fn increment_data_image_routine_ids(image: &mut NirDataImage) {
    for relocation in &mut image.relocations {
        if let NirDataRelocationTarget::Routine(id) = &mut relocation.target {
            *id = id.saturating_add(1);
        }
    }
}

fn numeric_initializer_values(expr: &SemExpr) -> Option<Vec<u16>> {
    match &expr.kind {
        SemExprKind::InitializerList(elements) => elements
            .iter()
            .map(sem_initializer_literal_value)
            .collect::<Option<Vec<_>>>(),
        SemExprKind::Raw(text) => {
            let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
            raw_initializer_values(inner)
        }
        _ => None,
    }
}

fn sem_initializer_literal_value(element: &SemInitializerElement) -> Option<u16> {
    let SemInitializerElementKind::Literal { value, negative } = &element.kind else {
        return None;
    };
    let value = match value {
        SemInitializerLiteral::Number(number) => number.value?,
        SemInitializerLiteral::Char(ch) => u16::from(source_char_byte(*ch)?),
        SemInitializerLiteral::True => 1,
        SemInitializerLiteral::False | SemInitializerLiteral::Nil => 0,
    };
    Some(if *negative {
        0u16.wrapping_sub(value)
    } else {
        value
    })
}

fn sem_static_initializer_literal_value(value: &SemStaticInitializerValue) -> Option<u16> {
    let SemStaticInitializerValue::Literal { value, negative } = value else {
        return None;
    };
    let value = match value {
        SemInitializerLiteral::Number(number) => number.value?,
        SemInitializerLiteral::Char(ch) => u16::from(source_char_byte(*ch)?),
        SemInitializerLiteral::True => 1,
        SemInitializerLiteral::False | SemInitializerLiteral::Nil => 0,
    };
    Some(if *negative {
        0u16.wrapping_sub(value)
    } else {
        value
    })
}

fn sem_static_initializer_real_value(
    value: &SemStaticInitializerValue,
) -> Option<crate::atari_real::AtariReal> {
    let SemStaticInitializerValue::Literal { value, negative } = value else {
        return None;
    };
    let magnitude = match value {
        SemInitializerLiteral::Number(number) if number.kind == crate::lexer::NumberKind::Real => {
            number.text.clone()
        }
        SemInitializerLiteral::Number(number) => number.value?.to_string(),
        SemInitializerLiteral::Char(ch) => source_char_byte(*ch)?.to_string(),
        SemInitializerLiteral::True => "1".to_string(),
        SemInitializerLiteral::False | SemInitializerLiteral::Nil => "0".to_string(),
    };
    let text = if *negative {
        format!("-{magnitude}")
    } else {
        magnitude
    };
    crate::atari_real::AtariReal::from_decimal(&text).ok()
}

fn sem_initializer_real_value(
    element: &SemInitializerElement,
) -> Option<crate::atari_real::AtariReal> {
    let SemInitializerElementKind::Literal { value, negative } = &element.kind else {
        return None;
    };
    let magnitude = match value {
        SemInitializerLiteral::Number(number) if number.kind == crate::lexer::NumberKind::Real => {
            number.text.clone()
        }
        SemInitializerLiteral::Number(number) => number.value?.to_string(),
        SemInitializerLiteral::Char(ch) => source_char_byte(*ch)?.to_string(),
        SemInitializerLiteral::True => "1".to_string(),
        SemInitializerLiteral::False | SemInitializerLiteral::Nil => "0".to_string(),
    };
    let text = if *negative {
        format!("-{magnitude}")
    } else {
        magnitude
    };
    crate::atari_real::AtariReal::from_decimal(&text).ok()
}

fn raw_initializer_values(inner: &str) -> Option<Vec<u16>> {
    let mut values = Vec::new();
    let mut sign = 1i32;
    for token in tokenize(inner).ok()? {
        match token.kind {
            TokenKind::Eof | TokenKind::Comma => continue,
            TokenKind::Plus => {
                sign = 1;
                continue;
            }
            TokenKind::Minus => {
                sign = -1;
                continue;
            }
            _ => {}
        }
        let raw = parse_raw_initializer_value(&token.kind)?;
        values.push(if sign < 0 {
            0u16.wrapping_sub(raw)
        } else {
            raw
        });
        sign = 1;
    }
    (!values.is_empty()).then_some(values)
}

fn parse_raw_initializer_value(token: &TokenKind) -> Option<u16> {
    match token {
        TokenKind::Number(number) => number.value,
        TokenKind::Char(ch) => source_char_byte(*ch).map(u16::from),
        TokenKind::Ident(name) => match storage_key(name).as_str() {
            "TRUE" => Some(1),
            "FALSE" | "NIL" => Some(0),
            _ => None,
        },
        _ => None,
    }
}

fn storage_key(name: &str) -> String {
    name.to_ascii_uppercase()
}

fn builtin_variable_address(name: &str) -> Option<u16> {
    resident_variable(name).map(|variable| variable.address)
}

fn resident_array_address(name: &str) -> Option<u16> {
    resident_variable(name).and_then(|variable| {
        matches!(variable.kind, ResidentVariableKind::ByteArray { .. }).then_some(variable.address)
    })
}

fn builtin_variable_type(name: &str) -> Option<NirType> {
    resident_variable(name).map(|variable| match variable.kind {
        ResidentVariableKind::Byte => NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(ByteSize::ONE),
            pointer: false,
        },
        ResidentVariableKind::ByteArray { .. } => pointer_type_to(&ValueType::fund(FundType::Byte)),
    })
}

fn pointer_type_to(pointee: &ValueType) -> NirType {
    let pointee_kind = NirTypeKind::from_value(pointee);
    NirType {
        kind: NirTypeKind::Pointer {
            pointee: Some(Box::new(pointee_kind)),
            address_space: TargetLayout::DATA_ADDRESS_SPACE,
        },
        summary: format!("{}*", type_summary(pointee)),
        width: Some(ByteSize::new(2)),
        pointer: true,
    }
}

fn descending_for_wrap_threshold(ty: &NirType, amount: u16) -> Option<u16> {
    match ty.kind {
        NirTypeKind::U8 if amount <= u16::from(u8::MAX) => Some(amount),
        NirTypeKind::I8 if amount <= 0x80 => Some((0x80 + amount) & 0x00FF),
        NirTypeKind::U16 => Some(amount),
        NirTypeKind::I16 if amount <= 0x8000 => Some(0x8000u16.wrapping_add(amount)),
        _ => None,
    }
}

fn ascending_for_wrap_threshold(ty: &NirType, amount: u16) -> Option<u16> {
    match ty.kind {
        NirTypeKind::U8 if amount <= u16::from(u8::MAX) => Some(u16::from(u8::MAX) - amount),
        NirTypeKind::I8 if amount <= 0x80 => Some(0x007F_u16.wrapping_sub(amount)),
        NirTypeKind::U16 => Some(u16::MAX - amount),
        NirTypeKind::I16 if amount <= 0x8000 => Some(0x7FFF_u16.wrapping_sub(amount)),
        _ => None,
    }
}

fn for_bound_is_at_or_above(ty: &NirType, bound: u16, threshold: u16) -> bool {
    match ty.kind {
        NirTypeKind::U8 => (bound as u8) >= threshold as u8,
        NirTypeKind::I8 => (bound as u8 as i8) >= threshold as u8 as i8,
        NirTypeKind::U16 => bound >= threshold,
        NirTypeKind::I16 => (bound as i16) >= threshold as i16,
        _ => false,
    }
}

fn for_bound_is_at_or_below(ty: &NirType, bound: u16, threshold: u16) -> bool {
    match ty.kind {
        NirTypeKind::U8 => (bound as u8) <= threshold as u8,
        NirTypeKind::I8 => (bound as u8 as i8) <= threshold as u8 as i8,
        NirTypeKind::U16 => bound <= threshold,
        NirTypeKind::I16 => (bound as i16) <= threshold as i16,
        _ => false,
    }
}

fn nir_scalar_constant(ty: &NirType, value: u16) -> NirValue {
    if ty.width == Some(ByteSize::ONE) {
        NirValue::ConstU8(value as u8)
    } else {
        NirValue::ConstU16(value)
    }
}

fn zero_value_for_type(ty: &ValueType) -> NirValue {
    let nir_ty = NirType::from_value(ty);
    if matches!(nir_ty.kind, NirTypeKind::Pointer { .. } | NirTypeKind::Callable { .. }) {
        NirValue::Null { ty: nir_ty }
    } else if ty.value_width_bytes() == Some(1) {
        NirValue::ConstU8(0)
    } else {
        NirValue::ConstU16(0)
    }
}

fn expr_summary(expr: &SemExpr) -> String {
    match &expr.kind {
        SemExprKind::Missing => "<missing>".to_string(),
        SemExprKind::Raw(raw) => raw.clone(),
        SemExprKind::InitializerList(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(|element| element.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        SemExprKind::UnresolvedName(name) => format!("unresolved({name})"),
        SemExprKind::CurrentLocation => "*".to_string(),
        SemExprKind::Literal(literal) => literal_summary(literal),
        SemExprKind::Symbol(symbol) => symbol.name.clone(),
        SemExprKind::LValue(lvalue) => lvalue_summary(lvalue),
        SemExprKind::AddressOf(lvalue) => format!("&{}", lvalue_summary(lvalue)),
        SemExprKind::AddressOfSymbol(symbol) => format!("&{}", symbol.name),
        SemExprKind::ImplicitAddressOf(address) => {
            format!(
                "&{} /* {:?} */",
                lvalue_summary(&address.place),
                address.reason
            )
        }
        SemExprKind::ArrayDecay(decay) => {
            format!("decay({})", lvalue_summary(&decay.array))
        }
        SemExprKind::Cast { ty, expr } => format!("cast({ty:?}, {})", expr_summary(expr)),
        SemExprKind::Unary { op, expr } => format!("{op:?} {}", expr_summary(expr)),
        SemExprKind::Binary { op, left, right } => {
            format!("{} {op:?} {}", expr_summary(left), expr_summary(right))
        }
        SemExprKind::Call(call) => {
            let args = call
                .args
                .iter()
                .map(expr_summary)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", callable_summary(&call.callee))
        }
    }
}

fn lvalue_summary(lvalue: &SemLValue) -> String {
    match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => symbol.name.clone(),
        SemLValueKind::UnresolvedName(name) => format!("unresolved({name})"),
        SemLValueKind::Deref { pointer } => format!("*{}", expr_summary(pointer)),
        SemLValueKind::Index {
            base,
            index,
            syntax,
            ..
        } => match syntax {
            crate::semantic::ir::SemIndexSyntax::Call => {
                format!("{}({})", expr_summary(base), expr_summary(index))
            }
            crate::semantic::ir::SemIndexSyntax::Index => {
                format!("{}[{}]", expr_summary(base), expr_summary(index))
            }
        },
        SemLValueKind::Field { base, field } => {
            format!("{}.{}", lvalue_summary(base), field.name)
        }
    }
}

fn callable_summary(callable: &SemCallable) -> String {
    match callable {
        SemCallable::User(symbol) | SemCallable::Builtin(symbol) => symbol.name.clone(),
        SemCallable::Indirect { target, .. } => format!("indirect({})", expr_summary(target)),
        SemCallable::Runtime { name, address, .. } => address
            .map(|address| format!("{name}@${address:04X}"))
            .unwrap_or_else(|| name.clone()),
    }
}

fn nir_machine_item(item: &MachineItem) -> Result<NirMachineItem, String> {
    Ok(match item {
        MachineItem::Number(number) => {
            let value = number
                .value
                .ok_or_else(|| machine_raw_item_diagnostic(&number.text))?;
            if let Ok(byte) = u8::try_from(value) {
                NirMachineItem::Byte(byte)
            } else {
                NirMachineItem::Word(value)
            }
        }
        MachineItem::StringLiteral(value) => NirMachineItem::StringLiteral(value.clone()),
        MachineItem::CharLiteral(value) => NirMachineItem::CharLiteral(*value),
        MachineItem::Name(name) => NirMachineItem::Name(name.to_string()),
        MachineItem::AddressExpr(expr) => NirMachineItem::AddressExpr {
            selector: expr.selector.map(nir_machine_byte_selector),
            explicit_address: expr.explicit_address,
            atom: nir_machine_atom(&expr.atom),
            offset: expr.offset,
            text: expr.text.clone(),
        },
        MachineItem::AddressByte { selector, name } => NirMachineItem::AddressByte {
            high: matches!(selector, AddressByteSelector::High),
            name: name.to_string(),
        },
        MachineItem::Raw(raw) => return Err(machine_raw_item_diagnostic(raw)),
    })
}

fn machine_raw_item_diagnostic(raw: &str) -> String {
    match raw {
        "+" | "-" => {
            format!(
                "machine block item `{raw}` is not a byte-stream item; use it only inside an address expression"
            )
        }
        _ if raw.starts_with('$') || raw.chars().next().is_some_and(|ch| ch.is_ascii_digit()) => {
            format!("machine block item `{raw}` does not fit in 16 bits")
        }
        _ => format!("unsupported raw machine block item `{raw}`"),
    }
}

fn machine_symbol_has_link_identity(symbol: &SemSymbolRef) -> bool {
    // Module-owned symbols have a stable declaration identity that the linker
    // can retarget for the selected runtime. Resident compatibility symbols
    // also have a synthesized external SYS declaration in legacy SemIR. Other
    // legacy root symbols retain their compatibility machine-item form until
    // all symbolic-offset forms carry structured addends.
    symbol.defining_module.is_some()
        || matches!(
            symbol.class,
            SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc
        )
}

fn nir_machine_atom(atom: &MachineAddressAtom) -> NirMachineAtom {
    match atom {
        MachineAddressAtom::Number(number) => number
            .value
            .map(NirMachineAtom::Number)
            .unwrap_or_else(|| NirMachineAtom::Name(number.text.clone())),
        MachineAddressAtom::Name(name) => NirMachineAtom::Name(name.to_string()),
        MachineAddressAtom::Current => NirMachineAtom::Current,
    }
}

fn nir_machine_byte_selector(selector: AddressByteSelector) -> NirMachineByteSelector {
    match selector {
        AddressByteSelector::Low => NirMachineByteSelector::Low,
        AddressByteSelector::High => NirMachineByteSelector::High,
    }
}

fn lvalue_is_param_symbol(lvalue: &SemLValue) -> bool {
    matches!(
        &lvalue.kind,
        SemLValueKind::Symbol(symbol)
            if matches!(symbol.class, crate::semantic::SymbolClass::Param)
    )
}

fn lvalue_symbol(lvalue: &SemLValue) -> Option<&SemSymbolRef> {
    match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => Some(symbol),
        _ => None,
    }
}

fn nir_call_signature(call: &SemCall) -> NirCallableSignature {
    NirCallableSignature {
        id: signature_id(&call.callable_type),
        params: call
            .callable_type
            .params
            .iter()
            .map(NirFacts::type_from_value)
            .collect(),
        variadic: call
            .callable_type
            .variadic
            .as_ref()
            .map(NirFacts::type_from_value),
        result: call
            .callable_type
            .return_type
            .as_ref()
            .map(NirFacts::type_from_value),
        kind: format!("{:?}", call.callable_type.kind),
        abi: "action".to_string(),
    }
}

fn nir_cast_kind(from: &NirType, to: &NirType) -> NirCastKind {
    match (from.kind.is_address(), to.kind.is_address()) {
        (true, true) => NirCastKind::Pointer,
        (false, true) => NirCastKind::IntegerToPointer,
        (true, false) => NirCastKind::PointerToInteger,
        (false, false) => NirCastKind::Integer,
    }
}

fn collect_memory_regions(
    regions: impl Iterator<Item = Option<NirMemoryRegion>>,
) -> NirMemoryAccess {
    let mut collected = Vec::new();
    for region in regions {
        let Some(region) = region else {
            return NirMemoryAccess::Unknown;
        };
        if !collected.contains(&region) {
            collected.push(region);
        }
    }
    if collected.is_empty() {
        NirMemoryAccess::None
    } else {
        NirMemoryAccess::Regions(collected)
    }
}

fn inclusive_region(kind: NirMemoryRegionKind, start: u16, end: u16) -> NirMemoryRegion {
    NirMemoryRegion {
        kind,
        offset: ByteOffset::from(start),
        size: ByteSize::from(
            end.checked_sub(start)
                .and_then(|size| size.checked_add(1))
                .unwrap_or(0),
        ),
    }
}

fn inline_asm_memory_access(
    regions: Option<Vec<NirMemoryRegion>>,
    otherwise_unknown: bool,
) -> NirMemoryAccess {
    if otherwise_unknown {
        return NirMemoryAccess::Unknown;
    }
    match regions {
        Some(regions) if regions.is_empty() => NirMemoryAccess::None,
        Some(regions) => NirMemoryAccess::Regions(regions),
        None => NirMemoryAccess::Unknown,
    }
}

fn inline_asm_regions(code: &NirInlineAsm, reads: bool) -> Option<Vec<NirMemoryRegion>> {
    let mut regions = Vec::new();
    for relocation in &code.relocations {
        let accesses = if reads {
            matches!(
                relocation.symbol_use,
                InlineAsmSymbolUse::Read
                    | InlineAsmSymbolUse::ReadWrite
                    | InlineAsmSymbolUse::IndexedRead
                    | InlineAsmSymbolUse::IndexedReadWrite
                    | InlineAsmSymbolUse::PointerRead
            )
        } else {
            matches!(
                relocation.symbol_use,
                InlineAsmSymbolUse::Write
                    | InlineAsmSymbolUse::ReadWrite
                    | InlineAsmSymbolUse::IndexedWrite
                    | InlineAsmSymbolUse::IndexedReadWrite
            )
        };
        if !accesses {
            continue;
        }
        match relocation.target {
            NirInlineAsmTarget::Storage(storage) => {
                let offset = u16::try_from(relocation.addend).ok()?;
                regions.push(NirMemoryRegion {
                    kind: NirMemoryRegionKind::Storage(storage),
                    offset: ByteOffset::from(offset),
                    size: if relocation.symbol_use == InlineAsmSymbolUse::PointerRead {
                        ByteSize::new(2)
                    } else {
                        ByteSize::ONE
                    },
                });
                continue;
            }
            NirInlineAsmTarget::Absolute(address) => {
                let address = address.checked_add_signed(i64::from(relocation.addend))?;
                let offset = u32::try_from(address.value).ok()?;
                regions.push(NirMemoryRegion {
                    kind: NirMemoryRegionKind::AbsoluteRange(address.address_space),
                    offset: ByteOffset::new(offset),
                    size: ByteSize::ONE,
                });
                continue;
            }
            NirInlineAsmTarget::Routine(_) | NirInlineAsmTarget::InlineOffset(_) => continue,
        }
    }
    regions.sort_by_key(|region| (format!("{:?}", region.kind), region.offset, region.size));
    regions.dedup();
    Some(regions)
}

fn literal_summary(literal: &SemLiteral) -> String {
    match literal {
        SemLiteral::Number(number) => number.text.clone(),
        SemLiteral::Real { source, .. } => source.text.clone(),
        SemLiteral::String(value) => format!("{value:?}"),
        SemLiteral::Char(value) => format!("{value:?}"),
        SemLiteral::Constant(value) => value.number_literal().text,
    }
}

fn is_real_value_type(ty: &ValueType) -> bool {
    matches!(ty.kind(), ValueTypeKind::Real)
}

fn is_real_nir_type(ty: &NirType) -> bool {
    matches!(ty.kind, NirTypeKind::Real)
}

fn real_nir_type() -> NirType {
    NirType {
        kind: NirTypeKind::Real,
        summary: "REAL".to_string(),
        width: Some(ByteSize::new(6)),
        pointer: false,
    }
}

fn literal_value(literal: &SemLiteral, ty: &NirType) -> Option<NirValue> {
    let value = match literal {
        SemLiteral::Number(number) => number.value?,
        SemLiteral::Real { .. } => return None,
        SemLiteral::Char(value) => *value as u16,
        SemLiteral::Constant(value) => value.bits,
        SemLiteral::String(_) => return None,
    };
    if ty.kind.is_pointer() {
        return Some(if value == 0 {
            NirValue::Null { ty: ty.clone() }
        } else {
            NirValue::AddressConst {
                address: AddressValue::new(pointer_address_space(ty)?, u64::from(value)),
                ty: ty.clone(),
            }
        });
    }
    match ty.width {
        Some(width) if width == ByteSize::ONE => {
            u8::try_from(value).ok().map(NirValue::ConstU8)
        }
        Some(width) if width == ByteSize::new(2) => Some(NirValue::ConstU16(value)),
        _ => None,
    }
}

fn pointer_address_space(ty: &NirType) -> Option<crate::target::AddressSpaceId> {
    match ty.kind {
        NirTypeKind::Pointer { address_space, .. }
        | NirTypeKind::Callable { address_space, .. } => Some(address_space),
        _ => None,
    }
}
fn array_origin_summary(origin: SemArrayOrigin) -> &'static str {
    match origin {
        SemArrayOrigin::Global => "global",
        SemArrayOrigin::Local => "local",
        SemArrayOrigin::Parameter => "parameter",
        SemArrayOrigin::RecordField => "record-field",
        SemArrayOrigin::Unknown => "unknown",
    }
}

fn sanitize_static_owner(owner: &str) -> String {
    let mut name = String::new();
    for ch in owner.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(ch);
        } else {
            name.push('_');
        }
    }
    if name.is_empty() {
        "program".to_string()
    } else {
        name
    }
}

fn is_runtime_helper_slot(address: u16) -> bool {
    matches!(address, 0x04E4 | 0x04E6 | 0x04E8 | 0x04EA | 0x04EC | 0x04EE)
}

#[cfg(test)]
mod memory_effect_tests {
    use super::*;
    use crate::semantic::{ScopeId, SymbolId as SemSymbolId};

    fn builder() -> NirBuilder {
        let mut global_ids = BTreeMap::new();
        global_ids.insert("g".to_string(), SymbolId(7));
        let mut storage_types = BTreeMap::new();
        storage_types.insert(
            "g".to_string(),
            NirType {
                kind: NirTypeKind::U16,
                summary: "Card".to_string(),
                width: Some(ByteSize::new(2)),
                pointer: false,
            },
        );
        let mut builder = NirBuilder::new(
            "Main",
            "bb0".to_string(),
            0,
            global_ids,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            storage_types,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            TargetLayout::atari_6502(),
        );
        builder.params.push(NirParam {
            id: ParamId(2),
            name: "p".to_string(),
            storage: NirStorageClass::Scalar,
            ty: NirType {
                kind: NirTypeKind::U8,
                summary: "Byte".to_string(),
                width: Some(ByteSize::ONE),
                pointer: false,
            },
        });
        builder.locals.push(NirLocal {
            id: LocalId(3),
            name: "x".to_string(),
            kind: "Byte".to_string(),
            purpose: NirLocalPurpose::Storage,
            storage: NirStorageClass::Scalar,
            ty: NirType {
                kind: NirTypeKind::U8,
                summary: "Byte".to_string(),
                width: Some(ByteSize::ONE),
                pointer: false,
            },
            backing: NirLocalBacking::Ordinary,
            init: None,
        });
        builder
            .local_ids_by_symbol
            .insert(SemSymbolId(9), LocalId(3));
        builder
    }

    #[test]
    fn semir_storage_effects_keep_exact_nir_storage_ids_and_ranges() {
        let builder = builder();
        let symbol = crate::semantic::ir::SemSymbolRef {
            id: SemSymbolId(9),
            name: "x".to_string(),
            defining_module: None,
            canonical_qualified_key: "X".to_string(),
            qualified_name: "x".to_string(),
            lexical_display_name: None,
            class: SymbolClass::Var,
            ty: None,
            is_volatile: false,
            scope: ScopeId(1),
            span: crate::source::Span::new(0, 1),
        };
        let access = builder.nir_write_effects(
            &[
                SemWriteEffect::Storage(SemStorageRef {
                    symbol: Some(symbol),
                    space: crate::semantic::ir::SemAddressSpace::RoutineLocal,
                    address: None,
                    offset: 0,
                    width: 1,
                    signed: false,
                    span: crate::source::Span::new(0, 1),
                }),
                SemWriteEffect::Absolute {
                    start: 0xD000,
                    end: 0xD003,
                },
            ],
            false,
        );

        assert_eq!(
            access,
            NirMemoryAccess::Regions(vec![
                NirMemoryRegion {
                    kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(3))),
                    offset: ByteOffset::ZERO,
                    size: ByteSize::ONE,
                },
                NirMemoryRegion {
                    kind: NirMemoryRegionKind::AbsoluteRange(
                        TargetLayout::DATA_ADDRESS_SPACE,
                    ),
                    offset: ByteOffset::new(0xD000),
                    size: ByteSize::new(4),
                },
            ])
        );
    }

    #[test]
    fn unresolved_semir_effect_regions_become_unknown() {
        assert_eq!(
            builder().nir_read_effects(&[SemReadEffect::Symbol("missing".to_string())], false),
            NirMemoryAccess::Unknown
        );
    }
}
