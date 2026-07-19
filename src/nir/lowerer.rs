use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{
    AddressByteSelector, BinaryOp, FundType, MachineAddressAtom, MachineItem, UnaryOp,
};
use crate::lexer::{TokenKind, tokenize};
use crate::resident::{ResidentVariableKind, resident_variable};
use crate::semantic::{
    ArrayType, SymbolClass, ValueType,
    ir::{
        SemArrayOrigin, SemCall, SemCallable, SemCondition, SemConditionKind, SemDeclaration,
        SemDeclarationStorage, SemEffects, SemExpr, SemExprKind, SemLValue, SemLValueKind,
        SemLiteral, SemProgram, SemSet, SemStmt,
    },
};
use crate::source::source_char_byte;

use super::classifier::NirClassifier;
use super::facts::{
    BlockId, LocalId, NirFacts, NirType, NirTypeKind, NirValue, ParamId, SymbolId, TempId,
    type_summary,
};
use super::ir::*;

#[derive(Default)]
pub(super) struct NirLowerer {
    next_label: usize,
    next_global: u32,
    next_static: u32,
    global_ids: BTreeMap<String, SymbolId>,
    storage_symbols: BTreeSet<String>,
    symbol_storage_types: BTreeMap<String, NirType>,
    absolute_globals: BTreeMap<String, u16>,
    absolute_array_element_bases: BTreeMap<String, u16>,
    absolute_array_value_addresses: BTreeMap<String, u16>,
    compatible_cursor: Option<u16>,
    machine_defines: BTreeMap<usize, Vec<MachineItem>>,
    machine_define_names: BTreeMap<String, Vec<MachineItem>>,
}

impl NirLowerer {
    pub(super) fn program(&mut self, program: &SemProgram) -> NirProgram {
        let mut globals = Vec::new();
        let mut statics = Vec::new();
        let mut routines = Vec::new();
        let mut top_level_ops = Vec::new();
        let mut top_level = Vec::new();

        self.collect_global_ids(program);
        let machine_defines = collect_machine_defines(program);
        self.machine_define_names = machine_defines.names;
        self.machine_defines = machine_defines.ids;
        let record_storage_sizes = record_storage_sizes(program);

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
                            storage_size: 0,
                            array: None,
                            init: None,
                            backing: NirGlobalBacking::Ordinary,
                        });
                    }
                    crate::semantic::ir::SemItem::Include(include) => {
                        let id = self.global_id(&include.path);
                        globals.push(NirGlobal {
                            id,
                            name: include.path.clone(),
                            kind: "include".to_string(),
                            ty: None,
                            storage_size: 0,
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
                        if let Some(op) = runtime_helper_set_op(set) {
                            top_level_ops.push(op);
                        } else {
                            top_level_ops.push(set_op(set));
                        }
                    }
                    crate::semantic::ir::SemItem::Declaration(declaration) => {
                        let id = self.global_id(&declaration.symbol.name);
                        let address_initializer = declaration
                            .initializer
                            .as_ref()
                            .and_then(|expr| self.const_u16_expr(expr));
                        if let Some(ty) =
                            declaration_symbol_storage_type(declaration, address_initializer)
                        {
                            self.symbol_storage_types
                                .insert(declaration.symbol.name.clone(), ty);
                        }
                        let alias_initializer = self.scalar_storage_alias_initializer(declaration);
                        let backing = self.declaration_backing(
                            declaration,
                            &record_storage_sizes,
                            address_initializer,
                            alias_initializer,
                        );
                        if let NirGlobalBacking::Absolute(address) = backing {
                            self.absolute_globals
                                .insert(storage_key(&declaration.symbol.name), address);
                            if declaration_is_array(declaration) {
                                self.absolute_array_element_bases
                                    .insert(storage_key(&declaration.symbol.name), address);
                            }
                        }
                        if let Some(address) = address_initializer
                            && declaration_array_address_initializer_uses_pointer_storage(
                                declaration,
                                &record_storage_sizes,
                            )
                        {
                            self.absolute_array_value_addresses
                                .insert(storage_key(&declaration.symbol.name), address);
                        }
                        globals.push(NirGlobal {
                            id,
                            name: declaration.symbol.name.clone(),
                            kind: declaration_kind(declaration),
                            ty: Some(NirFacts::type_from_value(&declaration.ty.value)),
                            storage_size: declaration_storage_size(
                                declaration,
                                &record_storage_sizes,
                                address_initializer,
                            ),
                            array: declaration_array_fact(
                                declaration,
                                &record_storage_sizes,
                                address_initializer,
                            ),
                            init: declaration_global_init(
                                id,
                                declaration,
                                &record_storage_sizes,
                                &backing,
                                address_initializer,
                            ),
                            backing,
                        });
                        self.storage_symbols.insert(declaration.symbol.name.clone());
                    }
                    crate::semantic::ir::SemItem::Routine(routine) => {
                        let mut builder = NirBuilder::new(
                            &routine.symbol.name,
                            self.next_block_label(),
                            self.next_static,
                            self.global_ids.clone(),
                            self.symbol_storage_types.clone(),
                            self.absolute_array_element_bases.clone(),
                            self.absolute_array_value_addresses.clone(),
                            record_storage_sizes.clone(),
                            self.machine_defines.clone(),
                            self.machine_define_names.clone(),
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
                            }
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
                        let mut local_alias_targets = BTreeMap::new();
                        for (index, local) in routine.locals.iter().enumerate() {
                            let address_initializer = local
                                .initializer
                                .as_ref()
                                .and_then(|expr| self.const_u16_expr(expr));
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
                                    .absolute_array_element_bases
                                    .insert(storage_key(&local.symbol.name), address);
                            }
                            if let Some(ty) =
                                declaration_symbol_storage_type(local, address_initializer)
                            {
                                builder
                                    .symbol_storage_types
                                    .insert(local.symbol.name.clone(), ty);
                            }
                            builder.locals.push(NirLocal {
                                id: LocalId(index as u32),
                                name: local.symbol.name.clone(),
                                kind: declaration_kind(local),
                                storage: declaration_storage_class(&local.storage),
                                ty: NirFacts::type_from_value(&local.ty.value),
                                init: declaration_local_init(
                                    local,
                                    &record_storage_sizes,
                                    &backing,
                                ),
                                backing,
                            });
                            local_alias_targets.insert(
                                storage_key(&local.symbol.name),
                                (LocalId(index as u32), local.symbol.name.clone()),
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
                self.symbol_storage_types.clone(),
                self.absolute_array_element_bases.clone(),
                self.absolute_array_value_addresses.clone(),
                record_storage_sizes.clone(),
                self.machine_defines.clone(),
                self.machine_define_names.clone(),
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
        }

        NirProgram {
            globals,
            statics,
            routines,
        }
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
                let name = match item {
                    crate::semantic::ir::SemItem::Define(define) => Some(&define.symbol.name),
                    crate::semantic::ir::SemItem::Include(include) => Some(&include.path),
                    crate::semantic::ir::SemItem::Declaration(declaration) => {
                        Some(&declaration.symbol.name)
                    }
                    crate::semantic::ir::SemItem::Routine(routine) => Some(&routine.symbol.name),
                    crate::semantic::ir::SemItem::Set(_)
                    | crate::semantic::ir::SemItem::Statement(_)
                    | crate::semantic::ir::SemItem::Unsupported { .. } => None,
                };
                if let Some(name) = name
                    && !self.global_ids.contains_key(name)
                {
                    let id = self.next_global_id();
                    self.global_ids.insert(name.clone(), id);
                }
            }
        }
    }

    fn declaration_backing(
        &mut self,
        declaration: &SemDeclaration,
        record_storage_sizes: &BTreeMap<String, u16>,
        address_initializer: Option<u16>,
        alias_initializer: Option<(String, u16)>,
    ) -> NirGlobalBacking {
        if let Some(address) = address_initializer {
            if declaration_array_address_initializer_uses_pointer_storage(
                declaration,
                record_storage_sizes,
            ) {
                return NirGlobalBacking::Ordinary;
            }
            return NirGlobalBacking::Absolute(address);
        }
        if let Some((target, offset)) = alias_initializer {
            return NirGlobalBacking::Alias { target, offset };
        }

        let Some(address) = self.compatible_cursor else {
            return NirGlobalBacking::Ordinary;
        };
        let size = declaration_storage_size(declaration, record_storage_sizes, address_initializer);
        self.compatible_cursor = Some(address.wrapping_add(size));
        NirGlobalBacking::Absolute(address)
    }

    fn scalar_storage_alias_initializer(
        &self,
        declaration: &SemDeclaration,
    ) -> Option<(String, u16)> {
        if !matches!(declaration.storage, SemDeclarationStorage::Scalar)
            || declaration.ty.value.pointer
        {
            return None;
        }
        let initializer = declaration.initializer.as_ref()?;
        let (target, offset) = storage_alias_initializer_expr(initializer)?;
        if !self.storage_symbols.contains(target) {
            return None;
        }
        Some((target.to_string(), offset))
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
        self.absolute_globals
            .insert(storage_key(&symbol.name), value);
        true
    }

    fn const_u16_expr(&self, expr: &SemExpr) -> Option<u16> {
        match &expr.kind {
            SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
            SemExprKind::Symbol(symbol) => self
                .absolute_globals
                .get(&storage_key(&symbol.name))
                .copied(),
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
        }
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
            SemLValueKind::Symbol(symbol) => self
                .absolute_globals
                .get(&storage_key(&symbol.name))
                .copied(),
            _ => None,
        }
    }

    fn local_backing(
        &self,
        declaration: &SemDeclaration,
        record_storage_sizes: &BTreeMap<String, u16>,
        address_initializer: Option<u16>,
        local_alias_targets: &BTreeMap<String, (LocalId, String)>,
    ) -> NirLocalBacking {
        if let Some(address) = address_initializer {
            match &declaration.storage {
                SemDeclarationStorage::Scalar if !declaration.ty.value.pointer => {
                    return NirLocalBacking::Absolute(address);
                }
                SemDeclarationStorage::Array { .. }
                    if !declaration_array_address_initializer_uses_pointer_storage(
                        declaration,
                        record_storage_sizes,
                    ) =>
                {
                    return NirLocalBacking::Absolute(address);
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
                offset,
            };
        }
        if matches!(declaration.storage, SemDeclarationStorage::Scalar)
            && !declaration.ty.value.pointer
            && let Some(initializer) = declaration.initializer.as_ref()
            && let Some((target_name, offset)) = storage_alias_initializer_expr(initializer)
            && self.storage_symbols.contains(target_name)
            && let Some(target) = self.global_ids.get(target_name).copied()
        {
            return NirLocalBacking::GlobalAlias {
                target,
                target_name: target_name.to_string(),
                offset,
            };
        }
        NirLocalBacking::Ordinary
    }
}

fn local_scalar_storage_alias_initializer(
    declaration: &SemDeclaration,
    local_alias_targets: &BTreeMap<String, (LocalId, String)>,
) -> Option<(LocalId, String, u16)> {
    if !matches!(declaration.storage, SemDeclarationStorage::Scalar) || declaration.ty.value.pointer
    {
        return None;
    }
    let initializer = declaration.initializer.as_ref()?;
    let (target, offset) = storage_alias_initializer_expr(initializer)?;
    let (target_id, target_name) = local_alias_targets.get(&storage_key(target))?;
    Some((*target_id, target_name.clone(), offset))
}

pub(super) struct NirBuilder {
    name: String,
    params: Vec<NirParam>,
    locals: Vec<NirLocal>,
    global_ids: BTreeMap<String, SymbolId>,
    symbol_storage_types: BTreeMap<String, NirType>,
    absolute_array_element_bases: BTreeMap<String, u16>,
    absolute_array_value_addresses: BTreeMap<String, u16>,
    record_storage_sizes: BTreeMap<String, u16>,
    machine_defines: BTreeMap<usize, Vec<MachineItem>>,
    machine_define_names: BTreeMap<String, Vec<MachineItem>>,
    notes: Vec<NirRoutineNote>,
    blocks: Vec<NirBlock>,
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
        symbol_storage_types: BTreeMap<String, NirType>,
        absolute_array_element_bases: BTreeMap<String, u16>,
        absolute_array_value_addresses: BTreeMap<String, u16>,
        record_storage_sizes: BTreeMap<String, u16>,
        machine_defines: BTreeMap<usize, Vec<MachineItem>>,
        machine_define_names: BTreeMap<String, Vec<MachineItem>>,
    ) -> Self {
        Self {
            name: name.to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            global_ids,
            symbol_storage_types,
            absolute_array_element_bases,
            absolute_array_value_addresses,
            record_storage_sizes,
            machine_defines,
            machine_define_names,
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: entry_label,
                ops: Vec::new(),
                terminator: NirTerminator::Open,
            }],
            current: 0,
            loop_exits: Vec::new(),
            next_block: 1,
            next_temp: 0,
            statics: Vec::new(),
            next_static,
        }
    }

    fn finish(mut self) -> (NirRoutine, Vec<NirStaticData>, u32) {
        self.resolve_storage_places();
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

    fn resolve_storage_places(&mut self) {
        let params = self
            .params
            .iter()
            .map(|param| (param.name.clone(), param.id))
            .collect::<BTreeMap<_, _>>();
        let locals = self
            .locals
            .iter()
            .filter(|local| {
                matches!(
                    local.backing,
                    NirLocalBacking::Ordinary | NirLocalBacking::Alias { .. }
                )
            })
            .map(|local| (local.name.clone(), local.id))
            .collect::<BTreeMap<_, _>>();
        let local_absolutes = self
            .locals
            .iter()
            .filter_map(|local| match local.backing {
                NirLocalBacking::Absolute(address) => Some((local.name.clone(), address)),
                NirLocalBacking::Ordinary
                | NirLocalBacking::Alias { .. }
                | NirLocalBacking::GlobalAlias { .. } => None,
            })
            .collect::<BTreeMap<_, _>>();
        let local_global_aliases = self
            .locals
            .iter()
            .filter_map(|local| match &local.backing {
                NirLocalBacking::GlobalAlias {
                    target,
                    target_name,
                    offset,
                } => Some((local.name.clone(), (*target, target_name.clone(), *offset))),
                NirLocalBacking::Ordinary
                | NirLocalBacking::Absolute(_)
                | NirLocalBacking::Alias { .. } => None,
            })
            .collect::<BTreeMap<_, _>>();
        let storage = StorageNameResolution {
            params,
            locals,
            local_absolutes,
            local_global_aliases,
            globals: self.global_ids.clone(),
        };

        for block in &mut self.blocks {
            for op in &mut block.ops {
                resolve_op_places(op, &storage);
            }
        }
    }

    fn push(&mut self, op: NirOp) {
        if !self.current_is_open() {
            let label = format!("{}.unreachable{}", self.name, self.blocks.len());
            self.start_block(label);
        }
        self.blocks[self.current].ops.push(op);
    }

    fn stmt_list(&mut self, statements: &[SemStmt], lowering: &mut NirLowerer) {
        for stmt in statements {
            self.stmt(stmt, lowering);
        }
    }

    fn stmt(&mut self, stmt: &SemStmt, lowering: &mut NirLowerer) {
        match stmt {
            SemStmt::Define(_) => {}
            SemStmt::Return { value, .. } => {
                let value = value.as_ref().map(|value| self.nir_value(value));
                self.terminate(NirTerminator::Return(value));
            }
            SemStmt::Exit { .. } => {
                if let Some(label) = self.loop_exits.last() {
                    self.terminate(NirTerminator::Goto(label.clone()));
                } else {
                    self.terminate(NirTerminator::Exit);
                }
            }
            SemStmt::Assign { target, value, .. } => {
                let fallback_ty = NirFacts::type_from_value(&target.ty);
                let target = self.lower_place(target);
                let target_ty = target.ty.clone().unwrap_or(fallback_ty);
                let value = self.value(value);
                self.assign_or_store(target, target_ty, value);
            }
            SemStmt::CompoundAssign {
                target, op, value, ..
            } => {
                let fallback_ty = NirFacts::type_from_value(&target.ty);
                let target = self.lower_place(target);
                let target_ty = target.ty.clone().unwrap_or(fallback_ty);
                let value = self.value(value);
                self.compound_or_legacy(target, target_ty, *op, value);
            }
            SemStmt::Call { call, .. } => {
                if let Some(items) = self.machine_define_call_items(call) {
                    self.push(NirOp::MachineBlock {
                        items,
                        effects: nir_machine_effects(&SemEffects::default()),
                    });
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
                    effects: nir_call_effects(&call.effects),
                });
            }
            SemStmt::MachineBlock { items, effects, .. } => {
                if items.is_empty() {
                    return;
                }
                self.push(NirOp::MachineBlock {
                    items: self.nir_machine_items(items),
                    effects: nir_machine_effects(effects),
                });
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
                    let condition = self.condition(&branch.condition);
                    self.terminate(NirTerminator::Branch {
                        condition,
                        then_label: body_label.clone(),
                        else_label: next_label.clone(),
                    });
                    self.start_block(body_label);
                    self.stmt_list(&branch.body, lowering);
                    self.finish_open_with(NirTerminator::Goto(after_label.clone()));
                    self.start_block(next_label);
                }
                if !else_body.is_empty() {
                    self.stmt_list(else_body, lowering);
                    self.finish_open_with(NirTerminator::Goto(after_label.clone()));
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
                self.finish_open_with(NirTerminator::Goto(test_label.clone()));
                self.start_block(test_label.clone());
                let condition = self.condition(condition);
                self.terminate(NirTerminator::Branch {
                    condition,
                    then_label: body_label.clone(),
                    else_label: after_label.clone(),
                });
                self.loop_exits.push(after_label.clone());
                self.start_block(body_label);
                self.stmt_list(body, lowering);
                self.finish_open_with(NirTerminator::Goto(test_label));
                self.loop_exits.pop();
                self.start_block(after_label);
            }
            SemStmt::DoUntil {
                body, condition, ..
            } => {
                let body_label = lowering.next_block_label();
                let after_label = lowering.next_block_label();
                self.finish_open_with(NirTerminator::Goto(body_label.clone()));
                self.loop_exits.push(after_label.clone());
                self.start_block(body_label.clone());
                self.stmt_list(body, lowering);
                if let Some(condition) = condition {
                    let condition = self.condition(condition);
                    self.finish_open_with(NirTerminator::Branch {
                        condition,
                        then_label: after_label.clone(),
                        else_label: body_label,
                    });
                } else {
                    self.finish_open_with(NirTerminator::Goto(body_label));
                }
                self.loop_exits.pop();
                self.start_block(after_label);
            }
            SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                ..
            } => {
                let target_ty = NirFacts::type_from_value(&target.ty);
                let target = self.lower_place(target);
                let test_label = lowering.next_block_label();
                let body_label = lowering.next_block_label();
                let after_label = lowering.next_block_label();
                let start = self.value(start);
                self.assign_or_store(target.clone(), target_ty.clone(), start);
                self.finish_open_with(NirTerminator::Goto(test_label.clone()));
                self.start_block(test_label.clone());
                let condition = self.for_limit_condition(&target, end);
                self.terminate(NirTerminator::Branch {
                    condition,
                    then_label: body_label.clone(),
                    else_label: after_label.clone(),
                });
                self.loop_exits.push(after_label.clone());
                self.start_block(body_label);
                self.stmt_list(body, lowering);
                let value = step
                    .as_ref()
                    .map(|step| self.value(step))
                    .unwrap_or_else(|| NirOperand {
                        kind: NirOperandKind::Literal {
                            text: "1".to_string(),
                            value: Some(1),
                        },
                        ty: Some(NirType {
                            kind: NirTypeKind::U8,
                            summary: "Byte".to_string(),
                            width: Some(1),
                            pointer: false,
                        }),
                    });
                self.compound_or_legacy(target, target_ty, BinaryOp::Add, value);
                self.finish_open_with(NirTerminator::Goto(test_label));
                self.loop_exits.pop();
                self.start_block(after_label);
            }
            SemStmt::Unsupported { note, .. } => {
                self.push(NirOp::Unsupported { note: note.clone() })
            }
        }
    }

    fn machine_define_call_items(&self, call: &SemCall) -> Option<Vec<NirMachineItem>> {
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

    fn nir_machine_items(&self, items: &[MachineItem]) -> Vec<NirMachineItem> {
        let mut lowered = Vec::new();
        let mut index = 0;
        while index < items.len() {
            let item = &items[index];
            if let Some((byte, split_item)) =
                self.split_compact_machine_number_item(item, items.get(index + 1))
            {
                lowered.push(NirMachineItem::Byte(byte));
                lowered.push(split_item);
                index += 2;
                continue;
            }
            if let MachineItem::Name(name) = item
                && let Some(items) = self.machine_define_names.get(&storage_key(name))
            {
                lowered.extend(items.iter().map(nir_machine_item));
                index += 1;
                continue;
            }
            lowered.push(nir_machine_item(item));
            index += 1;
        }
        lowered
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

    fn assign_or_store(&mut self, target: NirPlace, target_ty: NirType, value: NirOperand) {
        if let Some(src) = NirValue::from_legacy_operand(&value) {
            self.push(NirOp::Store {
                place: target,
                src,
                ty: target_ty,
            });
        } else {
            self.push(NirOp::Unsupported {
                note: "assignment source is not materialized".to_string(),
            });
        }
    }

    fn compound_or_legacy(
        &mut self,
        target: NirPlace,
        target_ty: NirType,
        op: BinaryOp,
        value: NirOperand,
    ) {
        let Some(src) = NirValue::from_legacy_operand(&value) else {
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
        self.push(NirOp::Load {
            dest: loaded,
            ty: target_ty.clone(),
            place: target.clone(),
        });

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

        self.push(NirOp::Store {
            place: target,
            src: NirValue::Temp {
                id: result,
                ty: target_ty.clone(),
            },
            ty: target_ty,
        });
    }

    fn value(&mut self, expr: &SemExpr) -> NirOperand {
        match &expr.kind {
            SemExprKind::Binary { op, left, right } if NirClassifier::is_nir_compare_op(*op) => {
                let left = self.nir_value(left);
                let right = self.nir_value(right);
                let dest = self.next_temp();
                let ty = NirFacts::condition_type();
                self.push(NirOp::Compare {
                    dest,
                    ty: ty.clone(),
                    op: NirClassifier::compare_op(*op)
                        .expect("compare-classified op should lower to NIR"),
                    left,
                    right,
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
            }
            SemExprKind::Cast { expr: inner, .. } => {
                let src = self.nir_value(inner);
                let dest = self.next_temp();
                let from = NirFacts::type_from_value(&inner.ty);
                let to = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Cast {
                    dest,
                    src,
                    from,
                    to: to.clone(),
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(to),
                }
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
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
            }
            SemExprKind::Binary { op, left, right } => {
                let left = self.nir_value(left);
                let right = self.nir_value(right);
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Binary {
                    dest,
                    ty: ty.clone(),
                    op: NirClassifier::binary_op(*op)
                        .expect("binary expression op should lower to NIR"),
                    left,
                    right,
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
            }
            SemExprKind::LValue(lvalue) => {
                let place = self.lower_place(lvalue);
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Load {
                    dest,
                    ty: ty.clone(),
                    place,
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
            }
            SemExprKind::Symbol(symbol) => {
                let dest = self.next_temp();
                let ty = self
                    .symbol_storage_type(&symbol.name)
                    .unwrap_or_else(|| NirFacts::type_from_value(&expr.ty));
                self.push(NirOp::Load {
                    dest,
                    ty: ty.clone(),
                    place: NirPlace {
                        kind: NirPlaceKind::Symbol(symbol.name.clone()),
                        ty: Some(ty.clone()),
                    },
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
            }
            SemExprKind::AddressOf(lvalue) => {
                let place = self.lower_place(lvalue);
                let ty = NirFacts::type_from_value(&expr.ty);
                self.addr_of_place(place, ty)
            }
            SemExprKind::AddressOfSymbol(symbol) => {
                let place_ty = symbol
                    .ty
                    .as_ref()
                    .map(NirType::from_value)
                    .or_else(|| Some(NirFacts::type_from_value(&expr.ty)));
                let place = NirPlace {
                    kind: NirPlaceKind::Symbol(symbol.name.clone()),
                    ty: place_ty,
                };
                let ty = NirFacts::type_from_value(&expr.ty);
                self.addr_of_place(place, ty)
            }
            SemExprKind::ImplicitAddressOf(address) => {
                let place = self.lower_place(&address.place);
                let ty = NirFacts::type_from_value(&expr.ty);
                self.addr_of_place(place, ty)
            }
            SemExprKind::ArrayDecay(decay) => {
                if decay.origin == SemArrayOrigin::Parameter
                    || self.lvalue_uses_pointer_storage(&decay.array)
                {
                    let place = self.lower_place(&decay.array);
                    let ty = NirFacts::type_from_value(&expr.ty);
                    return self.load_place_value(place, ty);
                }
                let place = self.lower_place(&decay.array);
                let ty = NirFacts::type_from_value(&expr.ty);
                self.addr_of_place(place, ty)
            }
            SemExprKind::Call(call) if NirClassifier::is_index_call_syntax(call) => {
                let place = self.lower_call_index_place(call, &expr.ty);
                let dest = self.next_temp();
                let ty = NirFacts::type_from_value(&expr.ty);
                self.push(NirOp::Load {
                    dest,
                    ty: ty.clone(),
                    place,
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
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
                    effects: nir_call_effects(&call.effects),
                });
                NirOperand {
                    kind: NirOperandKind::Temp(dest),
                    ty: Some(ty),
                }
            }
            _ => lower_operand(expr),
        }
    }

    fn nir_value(&mut self, expr: &SemExpr) -> NirValue {
        if let SemExprKind::Literal(SemLiteral::String(value)) = &expr.kind {
            return self.intern_string_literal(value, NirFacts::type_from_value(&expr.ty));
        }
        let operand = self.value(expr);
        NirValue::from_legacy_operand(&operand).expect("lowered NIR value operand should be simple")
    }

    fn intern_string_literal(&mut self, value: &str, ty: NirType) -> NirValue {
        let id = SymbolId(self.next_static);
        self.next_static += 1;
        let name = format!("__nir_str_{}_{}", sanitize_static_owner(&self.name), id.0);
        self.statics.push(NirStaticData {
            id,
            name: name.clone(),
            ty: ty.clone(),
            bytes: string_literal_storage_bytes(value)
                .unwrap_or_else(|_| value.as_bytes().to_vec()),
            display: value.to_string(),
            alignment: 1,
            mutable: false,
            section: "rodata".to_string(),
        });
        NirValue::StaticAddr { id, name, ty }
    }

    fn addr_of_place(&mut self, place: NirPlace, ty: NirType) -> NirOperand {
        if let NirPlaceKind::Symbol(name) = &place.kind
            && let Some(address) = self
                .absolute_array_value_addresses
                .get(&storage_key(name))
                .copied()
                .or_else(|| resident_array_address(name))
        {
            return NirOperand {
                kind: NirOperandKind::Literal {
                    text: format!("${address:04X}"),
                    value: Some(address),
                },
                ty: Some(ty),
            };
        }
        let dest = self.next_temp();
        self.push(NirOp::AddrOf {
            dest,
            ty: ty.clone(),
            place,
        });
        NirOperand {
            kind: NirOperandKind::Temp(dest),
            ty: Some(ty),
        }
    }

    fn load_place_value(&mut self, place: NirPlace, ty: NirType) -> NirOperand {
        let dest = self.next_temp();
        self.push(NirOp::Load {
            dest,
            ty: ty.clone(),
            place,
        });
        NirOperand {
            kind: NirOperandKind::Temp(dest),
            ty: Some(ty),
        }
    }

    fn lower_place(&mut self, lvalue: &SemLValue) -> NirPlace {
        let ty = self.lvalue_storage_type(lvalue);
        let kind = match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => {
                if let Some(address) = lvalue.storage.as_ref().and_then(|storage| storage.address) {
                    NirPlaceKind::Absolute(address)
                } else {
                    NirPlaceKind::Symbol(symbol.name.clone())
                }
            }
            SemLValueKind::UnresolvedName(name) => NirPlaceKind::UnresolvedName(name.clone()),
            SemLValueKind::Deref { pointer } => {
                let addr = self.nir_value(pointer);
                NirPlaceKind::Deref { addr }
            }
            SemLValueKind::Index {
                base,
                index,
                element_type,
                ..
            } => self.lower_index_place(base, index, element_type),
            SemLValueKind::Field { base, field } => NirPlaceKind::Field {
                base: Box::new(self.lower_place(base)),
                offset: field.offset.unwrap_or(0),
                ty: NirFacts::type_from_value(&field.ty),
            },
        };
        NirPlace { kind, ty }
    }

    fn lvalue_storage_type(&self, lvalue: &SemLValue) -> Option<NirType> {
        if let SemLValueKind::Symbol(symbol) = &lvalue.kind
            && let Some(ty) = self.symbol_storage_type(&symbol.name)
        {
            return Some(ty);
        }
        Some(NirFacts::type_from_value(&lvalue.ty))
    }

    fn symbol_storage_type(&self, name: &str) -> Option<NirType> {
        self.symbol_storage_types
            .get(name)
            .cloned()
            .or_else(|| builtin_variable_type(name))
    }

    fn lvalue_uses_pointer_storage(&self, lvalue: &SemLValue) -> bool {
        matches!(
            &lvalue.kind,
            SemLValueKind::Symbol(symbol) if self.symbol_storage_types.contains_key(&symbol.name)
        )
    }

    fn lower_index_place(
        &mut self,
        base: &SemExpr,
        index: &SemExpr,
        element_type: &ValueType,
    ) -> NirPlaceKind {
        let elem_ty = NirFacts::type_from_value(element_type);
        let elem_size = self.element_width(element_type).unwrap_or(1);
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
        let elem_size = self.element_width(ty).unwrap_or(1);
        let place = NirPlace {
            kind: NirPlaceKind::Symbol(symbol.name.clone()),
            ty: symbol.ty.as_ref().map(NirType::from_value),
        };
        let pointer_ty = pointer_type_to(ty);
        let base_addr = if matches!(symbol.class, crate::semantic::SymbolClass::Param) {
            NirValue::from_legacy_operand(&self.load_place_value(place, pointer_ty))
                .expect("parameter array index callee pointer load should produce a temp")
        } else if let Some(address) = self.absolute_index_base_for_name(&symbol.name) {
            NirValue::ConstU16(address)
        } else {
            NirValue::from_legacy_operand(&self.addr_of_place(place, pointer_ty))
                .expect("address-of index callee should produce a temp")
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

    fn element_width(&self, ty: &ValueType) -> Option<u16> {
        ty.value_width_bytes().or_else(|| {
            ty.as_record_name()
                .and_then(|name| self.record_storage_sizes.get(name).copied())
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
            let operand = self.load_place_value(place, pointer_ty);
            return NirValue::from_legacy_operand(&operand)
                .expect("parameter array pointer load should produce a temp");
        }
        if let SemExprKind::Symbol(symbol) = &base.kind
            && matches!(symbol.class, crate::semantic::SymbolClass::Param)
        {
            let pointer_ty = pointer_type_to(element_type);
            let place = NirPlace {
                kind: NirPlaceKind::Symbol(symbol.name.clone()),
                ty: Some(pointer_ty.clone()),
            };
            let operand = self.load_place_value(place, pointer_ty);
            return NirValue::from_legacy_operand(&operand)
                .expect("parameter array pointer load should produce a temp");
        }

        let place = match &base.kind {
            SemExprKind::Symbol(symbol) => NirPlace {
                kind: NirPlaceKind::Symbol(symbol.name.clone()),
                ty: symbol.ty.as_ref().map(NirType::from_value),
            },
            SemExprKind::LValue(lvalue) => self.lower_place(lvalue),
            _ => return self.nir_value(base),
        };
        let pointer_ty = pointer_type_to(element_type);
        let operand = self.addr_of_place(place, pointer_ty);
        NirValue::from_legacy_operand(&operand).expect("address-of place should produce a temp")
    }

    fn absolute_array_base_address(&self, base: &SemExpr) -> Option<u16> {
        let name = match &base.kind {
            SemExprKind::Symbol(symbol) => Some(symbol.name.as_str()),
            SemExprKind::LValue(lvalue) => lvalue_symbol_name(lvalue),
            SemExprKind::ArrayDecay(decay) => lvalue_symbol_name(&decay.array),
            _ => None,
        }?;
        self.absolute_index_base_for_name(name)
    }

    fn absolute_index_base_for_name(&self, name: &str) -> Option<u16> {
        let key = storage_key(name);
        self.absolute_array_value_addresses
            .get(&key)
            .or_else(|| self.absolute_array_element_bases.get(&key))
            .copied()
            .or_else(|| resident_array_address(name))
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

    fn nonzero_condition(&mut self, expr: &SemExpr) -> NirValue {
        let value = self.nir_value(expr);
        match value {
            NirValue::ConstU8(value) => NirValue::ConstU8(u8::from(value != 0)),
            NirValue::ConstU16(value) => NirValue::ConstU8(u8::from(value != 0)),
            value => {
                let dest = self.next_temp();
                let ty = NirFacts::condition_type();
                self.push(NirOp::Compare {
                    dest,
                    ty: ty.clone(),
                    op: NirCompareOp::Ne,
                    left: value,
                    right: zero_value_for_type(&expr.ty),
                });
                NirValue::Temp { id: dest, ty }
            }
        }
    }

    fn for_limit_condition(&mut self, target: &NirPlace, end: &SemExpr) -> NirValue {
        let left_ty = target.ty.clone().unwrap_or_else(NirFacts::condition_type);
        let left_temp = self.next_temp();
        self.push(NirOp::Load {
            dest: left_temp,
            ty: left_ty.clone(),
            place: target.clone(),
        });
        let right = self.nir_value(end);
        let dest = self.next_temp();
        let ty = NirFacts::condition_type();
        self.push(NirOp::Compare {
            dest,
            ty: ty.clone(),
            op: NirCompareOp::Le,
            left: NirValue::Temp {
                id: left_temp,
                ty: left_ty,
            },
            right,
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
            SemCallable::User(symbol) => NirCallee::User(symbol.name.clone()),
            SemCallable::Builtin(symbol) => NirCallee::Builtin(symbol.name.clone()),
            SemCallable::Indirect { target, .. } => NirCallee::Indirect {
                target: self.nir_value(target),
                ty: NirFacts::type_from_value(&target.ty),
            },
            SemCallable::Runtime { name, address, .. } => NirCallee::Runtime {
                name: name.clone(),
                address: *address,
            },
        }
    }

    fn terminate(&mut self, terminator: NirTerminator) {
        if !self.current_is_open() {
            let label = format!("{}.unreachable{}", self.name, self.blocks.len());
            self.start_block(label);
        }
        self.blocks[self.current].terminator = terminator;
    }

    fn finish_open_with(&mut self, terminator: NirTerminator) {
        if self.current_is_open() {
            self.blocks[self.current].terminator = terminator;
        }
    }

    fn current_is_open(&self) -> bool {
        matches!(self.blocks[self.current].terminator, NirTerminator::Open)
    }

    fn current_label(&self) -> &str {
        &self.blocks[self.current].label
    }

    fn start_block(&mut self, label: String) {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.blocks.push(NirBlock {
            id,
            label,
            ops: Vec::new(),
            terminator: NirTerminator::Open,
        });
        self.current = self.blocks.len() - 1;
    }
}

fn record_storage_sizes(program: &SemProgram) -> BTreeMap<String, u16> {
    program
        .modules
        .iter()
        .flat_map(|module| module.items.iter())
        .filter_map(|item| match item {
            crate::semantic::ir::SemItem::Declaration(declaration) => match &declaration.storage {
                SemDeclarationStorage::Type { record_type, .. }
                | SemDeclarationStorage::Record { record_type, .. } => {
                    Some((record_type.name.clone(), record_type.size))
                }
                SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. } => None,
            },
            _ => None,
        })
        .collect()
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
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
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
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
        | SemStmt::Unsupported { .. } => {}
    }
}

fn collect_machine_defines_from_stmt(stmt: &SemStmt, defines: &mut MachineDefines) {
    match stmt {
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
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
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
        for (op_index, op) in block.ops.iter().enumerate() {
            if let Some((id, ty)) = op_temp_def(op) {
                temps.push(NirTemp {
                    id,
                    ty: ty.clone(),
                    def: NirTempDef {
                        block: block.id,
                        op_index,
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
        | NirOp::AddrOf { dest, ty, .. }
        | NirOp::Unary { dest, ty, .. }
        | NirOp::Binary { dest, ty, .. }
        | NirOp::Compare { dest, ty, .. } => Some((*dest, ty)),
        NirOp::Cast { dest, to, .. } => Some((*dest, to)),
        NirOp::Call {
            result: Some(result),
            ..
        } => Some((result.dest, &result.ty)),
        NirOp::Define { .. }
        | NirOp::Set { .. }
        | NirOp::Declare { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. }
        | NirOp::Store { .. }
        | NirOp::Call { result: None, .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => None,
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
) -> u16 {
    match &declaration.storage {
        SemDeclarationStorage::Scalar => declaration
            .ty
            .value
            .value_width_bytes()
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
        ),
        SemDeclarationStorage::Type { .. } => 0,
        SemDeclarationStorage::Record { record_type, .. } => record_type.size,
    }
}

fn declaration_is_array(declaration: &SemDeclaration) -> bool {
    matches!(declaration.storage, SemDeclarationStorage::Array { .. })
}

fn storage_alias_initializer_expr(expr: &SemExpr) -> Option<(&str, u16)> {
    match &expr.kind {
        SemExprKind::Symbol(symbol) => Some((symbol.name.as_str(), 0)),
        SemExprKind::LValue(lvalue) => lvalue_symbol_name(lvalue).map(|name| (name, 0)),
        SemExprKind::ArrayDecay(decay) => lvalue_symbol_name(&decay.array).map(|name| (name, 0)),
        SemExprKind::Cast { expr, .. } => storage_alias_initializer_expr(expr),
        SemExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } => {
            let (name, base_offset) = storage_alias_initializer_expr(left)?;
            let offset = literal_expr_u16(right)?;
            Some((name, base_offset.wrapping_add(offset)))
        }
        _ => None,
    }
}

fn literal_expr_u16(expr: &SemExpr) -> Option<u16> {
    match &expr.kind {
        SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
        SemExprKind::Cast { expr, .. } => literal_expr_u16(expr),
        _ => None,
    }
}

fn declaration_array_storage_size(
    declaration: &SemDeclaration,
    array_type: &ArrayType,
    record_storage_sizes: &BTreeMap<String, u16>,
    address_initializer: Option<u16>,
) -> u16 {
    let elem_size = array_element_width(array_type, record_storage_sizes).unwrap_or(1);
    if array_type.length.is_none() && numeric_initializer_bytes(declaration, elem_size).is_some() {
        return 2;
    }
    if address_initializer.is_some()
        && declaration_array_address_initializer_uses_pointer_storage(
            declaration,
            record_storage_sizes,
        )
    {
        return if array_type.length.is_some() { 4 } else { 2 };
    }
    if elem_size > 1 && numeric_initializer_bytes(declaration, elem_size).is_some() {
        return if array_type.length.is_some() { 4 } else { 2 };
    }
    if elem_size == 1
        && let Some(bytes) = string_initializer_bytes(declaration)
            .or_else(|| numeric_initializer_bytes(declaration, elem_size))
    {
        return array_type
            .length
            .map(|length| length.saturating_mul(elem_size))
            .unwrap_or(bytes.len() as u16)
            .max(bytes.len() as u16);
    }
    array_type
        .length
        .map(|length| length.saturating_mul(elem_size))
        .unwrap_or(2)
}

fn declaration_array_address_initializer_uses_pointer_storage(
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
) -> bool {
    let SemDeclarationStorage::Array { array_type, .. } = &declaration.storage else {
        return false;
    };
    if declaration.initializer.is_none() {
        return false;
    }
    let elem_size = array_element_width(array_type, record_storage_sizes).unwrap_or(1);
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
            if matches!(symbol.class, SymbolClass::Proc | SymbolClass::Func) =>
        {
            Some(symbol.name.clone())
        }
        _ => None,
    }
}

fn array_element_width(
    array_type: &ArrayType,
    record_storage_sizes: &BTreeMap<String, u16>,
) -> Option<u16> {
    array_type.element_width_bytes().or_else(|| {
        array_type
            .element
            .as_record_name()
            .and_then(|name| record_storage_sizes.get(name).copied())
    })
}

fn declaration_array_fact(
    declaration: &SemDeclaration,
    record_storage_sizes: &BTreeMap<String, u16>,
    address_initializer: Option<u16>,
) -> Option<NirArrayGlobalFact> {
    let SemDeclarationStorage::Array { array_type, .. } = &declaration.storage else {
        return None;
    };
    let elem_size = array_element_width(array_type, record_storage_sizes).unwrap_or(1);
    Some(NirArrayGlobalFact {
        elem_size,
        length: array_type.length,
        pointer_backed: array_type.length.is_none()
            || (address_initializer.is_some()
                && declaration_array_address_initializer_uses_pointer_storage(
                    declaration,
                    record_storage_sizes,
                )),
        address_initializer,
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
) -> Option<NirGlobalInit> {
    if matches!(backing, NirGlobalBacking::Absolute(_)) {
        return None;
    }
    let storage_size =
        declaration_storage_size(declaration, record_storage_sizes, address_initializer);
    match &declaration.storage {
        SemDeclarationStorage::Scalar => scalar_initializer_bytes(declaration, storage_size)
            .map(|bytes| bytes_init(bytes, storage_size)),
        SemDeclarationStorage::Array { array_type, .. } => {
            let elem_size = array_element_width(array_type, record_storage_sizes).unwrap_or(1);
            if let Some(address) = address_initializer
                && declaration_array_address_initializer_uses_pointer_storage(
                    declaration,
                    record_storage_sizes,
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
                    name,
                    descriptor_size: if array_type.length.is_some() { 4 } else { 2 },
                    size_word: None,
                    mutable: true,
                    section: "global".to_string(),
                });
            }
            if elem_size > 1
                && let Some(bytes) = numeric_initializer_bytes(declaration, elem_size)
            {
                let len = array_type
                    .length
                    .unwrap_or((bytes.len() as u16) / elem_size);
                let byte_size = elem_size.saturating_mul(len).max(bytes.len() as u16);
                return Some(NirGlobalInit::Descriptor {
                    backing: NirDataBacking {
                        owner: id,
                        zero_fill: byte_size.saturating_sub(bytes.len() as u16),
                        bytes,
                        section: "global.backing".to_string(),
                    },
                    descriptor_size: if array_type.length.is_some() { 4 } else { 2 },
                    size_word: array_type.length.map(|_| 0),
                    mutable: true,
                    section: "global".to_string(),
                });
            }
            if array_type.length.is_none()
                && elem_size == 1
                && let Some(bytes) = numeric_initializer_bytes(declaration, elem_size)
            {
                return Some(NirGlobalInit::Descriptor {
                    backing: NirDataBacking {
                        owner: id,
                        zero_fill: 0,
                        bytes,
                        section: "global.backing".to_string(),
                    },
                    descriptor_size: 2,
                    size_word: None,
                    mutable: true,
                    section: "global".to_string(),
                });
            }
            let bytes = if elem_size == 1 {
                string_initializer_bytes(declaration)
                    .or_else(|| numeric_initializer_bytes(declaration, elem_size))
            } else {
                numeric_initializer_bytes(declaration, elem_size)
            };
            if let Some(bytes) = bytes {
                let total_size = array_type
                    .length
                    .map(|length| length.saturating_mul(elem_size))
                    .unwrap_or(bytes.len() as u16)
                    .max(bytes.len() as u16);
                return Some(bytes_init(bytes, total_size));
            }
            array_type.length.map(|length| {
                let bytes = length.saturating_mul(elem_size);
                NirGlobalInit::ZeroFill {
                    bytes,
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
    let zero_fill = total_size.saturating_sub(bytes.len() as u16);
    NirGlobalInit::Bytes {
        bytes,
        zero_fill,
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
    if matches!(global.backing, NirGlobalBacking::Absolute(_)) || global.storage_size < 2 {
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
) -> Option<NirStorageInit> {
    if matches!(
        backing,
        NirLocalBacking::Absolute(_)
            | NirLocalBacking::Alias { .. }
            | NirLocalBacking::GlobalAlias { .. }
    ) {
        return None;
    }
    let storage_size = declaration_storage_size(declaration, record_storage_sizes, None);
    match &declaration.storage {
        SemDeclarationStorage::Scalar => {
            if let Some(bytes) = scalar_initializer_bytes(declaration, storage_size) {
                return Some(storage_bytes_init(bytes, storage_size));
            }
            if storage_size > declaration.ty.value.value_width_bytes().unwrap_or(0) {
                return Some(NirStorageInit::ZeroFill {
                    bytes: storage_size,
                    mutable: true,
                    section: "local".to_string(),
                });
            }
            None
        }
        SemDeclarationStorage::Array { array_type, .. } => {
            let elem_size = array_element_width(array_type, record_storage_sizes).unwrap_or(1);
            if elem_size > 1
                && let Some(bytes) = numeric_initializer_bytes(declaration, elem_size)
            {
                let len = array_type
                    .length
                    .unwrap_or((bytes.len() as u16) / elem_size);
                let byte_size = elem_size.saturating_mul(len).max(bytes.len() as u16);
                return Some(NirStorageInit::Descriptor {
                    backing: NirStorageBacking {
                        zero_fill: byte_size.saturating_sub(bytes.len() as u16),
                        bytes,
                        section: "local.backing".to_string(),
                    },
                    descriptor_size: if array_type.length.is_some() { 4 } else { 2 },
                    size_word: array_type.length.map(|_| 0),
                    mutable: true,
                    section: "local".to_string(),
                });
            }
            let bytes = if elem_size == 1 {
                string_initializer_bytes(declaration)
                    .or_else(|| numeric_initializer_bytes(declaration, elem_size))
            } else {
                numeric_initializer_bytes(declaration, elem_size)
            };
            if let Some(bytes) = bytes {
                let total_size = array_type
                    .length
                    .map(|length| length.saturating_mul(elem_size))
                    .unwrap_or(bytes.len() as u16)
                    .max(bytes.len() as u16);
                return Some(storage_bytes_init(bytes, total_size));
            }
            array_type.length.map(|length| {
                let bytes = length.saturating_mul(elem_size);
                NirStorageInit::ZeroFill {
                    bytes,
                    mutable: true,
                    section: "local".to_string(),
                }
            })
        }
        SemDeclarationStorage::Record { .. } | SemDeclarationStorage::Type { .. } => None,
    }
}

fn storage_bytes_init(bytes: Vec<u8>, total_size: u16) -> NirStorageInit {
    let zero_fill = total_size.saturating_sub(bytes.len() as u16);
    NirStorageInit::Bytes {
        bytes,
        zero_fill,
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

fn numeric_initializer_values(expr: &SemExpr) -> Option<Vec<u16>> {
    let SemExprKind::Raw(text) = &expr.kind else {
        return None;
    };
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    raw_initializer_values(inner)
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
            width: Some(1),
            pointer: false,
        },
        ResidentVariableKind::ByteArray { .. } => pointer_type_to(&ValueType::fund(FundType::Byte)),
    })
}

fn lower_operand(expr: &SemExpr) -> NirOperand {
    let ty = Some(NirFacts::type_from_value(&expr.ty));
    let kind = match &expr.kind {
        SemExprKind::Missing => NirOperandKind::Missing,
        SemExprKind::Raw(raw) => NirOperandKind::Raw(raw.clone()),
        SemExprKind::UnresolvedName(name) => NirOperandKind::UnresolvedName(name.clone()),
        SemExprKind::CurrentLocation => NirOperandKind::CurrentLocation,
        SemExprKind::Literal(literal) => literal_operand_kind(literal),
        SemExprKind::Symbol(symbol) => NirOperandKind::Symbol(symbol.name.clone()),
        SemExprKind::LValue(lvalue) => NirOperandKind::Place(Box::new(lower_legacy_place(lvalue))),
        SemExprKind::AddressOf(lvalue) => {
            NirOperandKind::AddressOf(Box::new(lower_legacy_place(lvalue)))
        }
        SemExprKind::AddressOfSymbol(symbol) => {
            NirOperandKind::AddressOfSymbol(symbol.name.clone())
        }
        SemExprKind::ImplicitAddressOf(address) => {
            NirOperandKind::AddressOf(Box::new(lower_legacy_place(&address.place)))
        }
        SemExprKind::Call(call) => NirOperandKind::Call(format!(
            "{}({})",
            callable_summary(&call.callee),
            call.args
                .iter()
                .map(expr_summary)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        SemExprKind::ArrayDecay(_)
        | SemExprKind::Cast { .. }
        | SemExprKind::Unary { .. }
        | SemExprKind::Binary { .. } => NirOperandKind::Expr(expr_summary(expr)),
    };
    NirOperand { kind, ty }
}

struct StorageNameResolution {
    params: BTreeMap<String, ParamId>,
    locals: BTreeMap<String, LocalId>,
    local_absolutes: BTreeMap<String, u16>,
    local_global_aliases: BTreeMap<String, (SymbolId, String, u16)>,
    globals: BTreeMap<String, SymbolId>,
}

fn resolve_op_places(op: &mut NirOp, storage: &StorageNameResolution) {
    match op {
        NirOp::Assign { target, value } => {
            resolve_place_storage(target, storage);
            resolve_operand_places(value, storage);
        }
        NirOp::CompoundAssign { target, value, .. } => {
            resolve_place_storage(target, storage);
            resolve_operand_places(value, storage);
        }
        NirOp::Load { place, .. } | NirOp::AddrOf { place, .. } | NirOp::Store { place, .. } => {
            resolve_place_storage(place, storage);
        }
        NirOp::Set { address, value } => {
            resolve_operand_places(address, storage);
            resolve_operand_places(value, storage);
        }
        NirOp::Define { .. }
        | NirOp::Declare { .. }
        | NirOp::Unary { .. }
        | NirOp::Cast { .. }
        | NirOp::Binary { .. }
        | NirOp::Compare { .. }
        | NirOp::Call { .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => {}
    }
}

fn resolve_operand_places(operand: &mut NirOperand, storage: &StorageNameResolution) {
    match &mut operand.kind {
        NirOperandKind::Place(place) | NirOperandKind::AddressOf(place) => {
            resolve_place_storage(place, storage);
        }
        NirOperandKind::Missing
        | NirOperandKind::Raw(_)
        | NirOperandKind::UnresolvedName(_)
        | NirOperandKind::CurrentLocation
        | NirOperandKind::Literal { .. }
        | NirOperandKind::Temp(_)
        | NirOperandKind::Symbol(_)
        | NirOperandKind::AddressOfSymbol(_)
        | NirOperandKind::Expr(_)
        | NirOperandKind::Call(_) => {}
    }
}

fn resolve_place_storage(place: &mut NirPlace, storage: &StorageNameResolution) {
    match &mut place.kind {
        NirPlaceKind::Symbol(name) => {
            if let Some(address) = storage.local_absolutes.get(name).copied() {
                place.kind = NirPlaceKind::Absolute(address);
            } else if let Some((id, target_name, offset)) =
                storage.local_global_aliases.get(name).cloned()
            {
                let ty = place.ty.clone().unwrap_or(NirType {
                    kind: NirTypeKind::U8,
                    summary: "Byte".to_string(),
                    width: Some(1),
                    pointer: false,
                });
                place.kind = NirPlaceKind::Field {
                    base: Box::new(NirPlace {
                        kind: NirPlaceKind::Global {
                            id,
                            name: target_name,
                        },
                        ty: Some(ty.clone()),
                    }),
                    offset,
                    ty,
                };
            } else if let Some(id) = storage.locals.get(name).copied() {
                place.kind = NirPlaceKind::Local {
                    id,
                    name: name.clone(),
                };
            } else if let Some(id) = storage.params.get(name).copied() {
                place.kind = NirPlaceKind::Param {
                    id,
                    name: name.clone(),
                };
            } else if let Some(id) = storage.globals.get(name).copied() {
                place.kind = NirPlaceKind::Global {
                    id,
                    name: name.clone(),
                };
            } else if let Some(address) = builtin_variable_address(name) {
                place.kind = NirPlaceKind::Absolute(address);
            }
        }
        NirPlaceKind::Deref { .. } => {}
        NirPlaceKind::Index { .. } => {}
        NirPlaceKind::Field { base, .. } => resolve_place_storage(base, storage),
        NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_)
        | NirPlaceKind::UnresolvedName(_) => {}
    }
}

fn lower_legacy_place(lvalue: &SemLValue) -> NirPlace {
    let ty = Some(NirFacts::type_from_value(&lvalue.ty));
    let kind = match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => {
            if let Some(address) = lvalue.storage.as_ref().and_then(|storage| storage.address) {
                NirPlaceKind::Absolute(address)
            } else {
                NirPlaceKind::Symbol(symbol.name.clone())
            }
        }
        SemLValueKind::UnresolvedName(name) => NirPlaceKind::UnresolvedName(name.clone()),
        SemLValueKind::Deref { pointer } => {
            let operand = lower_operand(pointer);
            let addr = NirValue::from_legacy_operand(&operand).unwrap_or(NirValue::ConstU16(0));
            NirPlaceKind::Deref { addr }
        }
        SemLValueKind::Index {
            base,
            index,
            element_type,
            ..
        } => NirPlaceKind::Index {
            base_addr: NirValue::from_legacy_operand(&lower_operand(base))
                .unwrap_or(NirValue::ConstU16(0)),
            index: NirValue::from_legacy_operand(&lower_operand(index))
                .unwrap_or(NirValue::ConstU8(0)),
            elem_ty: NirFacts::type_from_value(element_type),
            elem_size: NirFacts::type_from_value(element_type).width.unwrap_or(1),
        },
        SemLValueKind::Field { base, field } => NirPlaceKind::Field {
            base: Box::new(lower_legacy_place(base)),
            offset: field.offset.unwrap_or(0),
            ty: NirFacts::type_from_value(&field.ty),
        },
    };
    NirPlace { kind, ty }
}

fn pointer_type_to(pointee: &ValueType) -> NirType {
    let pointee_kind = NirTypeKind::from_value(pointee);
    NirType {
        kind: NirTypeKind::Ptr16 {
            pointee: Some(Box::new(pointee_kind)),
        },
        summary: format!("{}*", type_summary(pointee)),
        width: Some(2),
        pointer: true,
    }
}

fn zero_value_for_type(ty: &ValueType) -> NirValue {
    if ty.value_width_bytes() == Some(1) {
        NirValue::ConstU8(0)
    } else {
        NirValue::ConstU16(0)
    }
}

fn expr_summary(expr: &SemExpr) -> String {
    match &expr.kind {
        SemExprKind::Missing => "<missing>".to_string(),
        SemExprKind::Raw(raw) => raw.clone(),
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

fn nir_call_effects(effects: &SemEffects) -> NirCallEffects {
    NirCallEffects {
        memory: NirMemoryEffects {
            reads: nir_memory_access(effects.reads.len(), effects.opaque),
            writes: nir_memory_access(effects.writes.len(), effects.opaque),
        },
        may_call_os: effects.may_call_os,
        opaque: effects.opaque,
    }
}

fn nir_machine_effects(effects: &SemEffects) -> NirMachineEffects {
    NirMachineEffects {
        memory: NirMemoryEffects {
            reads: nir_memory_access(effects.reads.len(), effects.opaque),
            writes: nir_memory_access(effects.writes.len(), effects.opaque),
        },
        may_call_os: effects.may_call_os,
        opaque: true,
    }
}

fn nir_machine_item(item: &MachineItem) -> NirMachineItem {
    match item {
        MachineItem::Number(number) => number
            .value
            .map(|value| {
                if let Ok(byte) = u8::try_from(value) {
                    NirMachineItem::Byte(byte)
                } else {
                    NirMachineItem::Word(value)
                }
            })
            .unwrap_or_else(|| NirMachineItem::Raw(number.text.clone())),
        MachineItem::StringLiteral(value) => NirMachineItem::StringLiteral(value.clone()),
        MachineItem::CharLiteral(value) => NirMachineItem::CharLiteral(*value),
        MachineItem::Name(name) => NirMachineItem::Name(name.clone()),
        MachineItem::AddressExpr(expr) => NirMachineItem::AddressExpr {
            selector: expr.selector.map(nir_machine_byte_selector),
            explicit_address: expr.explicit_address,
            atom: nir_machine_atom(&expr.atom),
            offset: expr.offset,
            text: expr.text.clone(),
        },
        MachineItem::AddressByte { selector, name } => NirMachineItem::AddressByte {
            high: matches!(selector, AddressByteSelector::High),
            name: name.clone(),
        },
        MachineItem::Raw(raw) => NirMachineItem::Raw(raw.clone()),
    }
}

fn nir_machine_atom(atom: &MachineAddressAtom) -> NirMachineAtom {
    match atom {
        MachineAddressAtom::Number(number) => number
            .value
            .map(NirMachineAtom::Number)
            .unwrap_or_else(|| NirMachineAtom::Name(number.text.clone())),
        MachineAddressAtom::Name(name) => NirMachineAtom::Name(name.clone()),
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

fn lvalue_symbol_name(lvalue: &SemLValue) -> Option<&str> {
    match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => Some(symbol.name.as_str()),
        _ => None,
    }
}

fn nir_call_signature(call: &SemCall) -> NirCallableSignature {
    NirCallableSignature {
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

fn nir_memory_access(regions: usize, opaque: bool) -> NirMemoryAccess {
    if opaque {
        NirMemoryAccess::Unknown
    } else if regions == 0 {
        NirMemoryAccess::None
    } else {
        NirMemoryAccess::Known { regions }
    }
}

fn literal_summary(literal: &SemLiteral) -> String {
    match literal {
        SemLiteral::Number(number) => number.text.clone(),
        SemLiteral::String(value) => format!("{value:?}"),
        SemLiteral::Char(value) => format!("{value:?}"),
    }
}

fn literal_operand_kind(literal: &SemLiteral) -> NirOperandKind {
    match literal {
        SemLiteral::Number(number) => NirOperandKind::Literal {
            text: number.text.clone(),
            value: number.value,
        },
        SemLiteral::String(value) => NirOperandKind::Literal {
            text: format!("{value:?}"),
            value: None,
        },
        SemLiteral::Char(value) => NirOperandKind::Literal {
            text: format!("{value:?}"),
            value: Some(*value as u16),
        },
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

fn set_op(set: &SemSet) -> NirOp {
    let address = lower_operand(&set.address);
    let value = lower_operand(&set.value);
    let Some(address) = literal_u16(&address) else {
        return NirOp::Unsupported {
            note: "SET address is not a numeric absolute address".to_string(),
        };
    };
    let Some(src) = NirValue::from_legacy_operand(&value) else {
        return NirOp::Unsupported {
            note: "SET value is not materialized".to_string(),
        };
    };
    let Some(ty) = value.ty.clone() else {
        return NirOp::Unsupported {
            note: "SET value has no NIR type".to_string(),
        };
    };
    NirOp::Store {
        place: NirPlace {
            kind: NirPlaceKind::Absolute(address),
            ty: Some(ty.clone()),
        },
        src,
        ty,
    }
}

fn runtime_helper_set_op(set: &SemSet) -> Option<NirOp> {
    let address = lower_operand(&set.address);
    let value = lower_operand(&set.value);
    let address_value = literal_u16(&address)?;
    if !is_runtime_helper_slot(address_value) {
        return None;
    }
    match value.kind {
        NirOperandKind::Symbol(_) | NirOperandKind::AddressOfSymbol(_) => {
            Some(NirOp::Set { address, value })
        }
        _ => None,
    }
}

fn is_runtime_helper_slot(address: u16) -> bool {
    matches!(address, 0x04E4 | 0x04E6 | 0x04E8 | 0x04EA | 0x04EC | 0x04EE)
}

fn literal_u16(operand: &NirOperand) -> Option<u16> {
    match operand.kind {
        NirOperandKind::Literal {
            value: Some(value), ..
        } => Some(value),
        _ => None,
    }
}
