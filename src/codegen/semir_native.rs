use std::collections::HashMap;

use crate::ast::{
    AddressByteSelector, BinaryOp, MachineAddressAtom, MachineAddressExpr, MachineItem, UnaryOp,
    machine_address_symbolic_offset,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{TokenKind, tokenize};
use crate::resident::{ResidentVariableKind, resident_variable};
use crate::semantic::ir::*;
use crate::semantic::{
    ArrayType, FieldId, RecordType, ScalarSignedness, SymbolClass, SymbolId, ValueType,
};
use crate::source::{Span, source_char_byte};

use super::*;

pub(super) fn generate_native_profile_with_origin(
    program: &SemProgram,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let model = SemIrReadModel::new(program, origin, profile);
    match SemIrNativeEmitter::new(&model).emit() {
        Ok(output) => Ok(output),
        Err(reason) => Err(vec![Diagnostic::new(
            model.first_span().unwrap_or_else(|| Span::new(0, 0)),
            format!(
                "native SemIR codegen is not implemented yet ({reason}; {})",
                model.summary()
            ),
        )]),
    }
}

struct SemIrNativeEmitter<'a, 'm> {
    model: &'m SemIrReadModel<'a>,
    storage: HashMap<SymbolId, NativeStorageSlot>,
    machine_caret_values: HashMap<SymbolId, u16>,
    emitter: NativeTrackedEmitter,
    routine_addresses: Vec<RoutineAddress>,
    routine_ranges: Vec<RoutineRange>,
    routine_entries: HashMap<SymbolId, u16>,
    routine_labels: HashMap<SymbolId, String>,
    runtime_helpers: RuntimeHelperTargets,
    storage_symbols: Vec<CodegenStorageSymbol>,
    record_layouts: HashMap<String, NativeRecordLayout>,
    record_fields_by_id: HashMap<FieldId, NativeRecordField>,
    source_ranges: Vec<CodegenSourceRange>,
    array_backings: Vec<NativeArrayBacking>,
    external_storage_cursor: Option<u16>,
    main_run_address: Option<u16>,
    last_routine_run_address: Option<u16>,
    label_counter: usize,
    y_known_zero: bool,
    current_return_width: Option<u16>,
    current_routine: Option<SymbolId>,
    exit_labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeStorageSlot {
    address: u16,
    width: u16,
    array: Option<NativeArrayStorage>,
    pointee_width: Option<u16>,
    record: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeArrayStorage {
    element_width: u16,
    len: u16,
    storage: CodegenArrayStorage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeArrayFacts {
    element_width: u16,
    declared_len: Option<u16>,
    record: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeArrayBacking {
    label: String,
    size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeResolvedSlot {
    address: u16,
    width: u16,
    pointee_width: Option<u16>,
    record: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeRecordLayout {
    record_type: RecordType,
    size: u16,
    fields: HashMap<String, NativeRecordField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeRecordField {
    offset: u16,
    width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeCallArgByte<'a> {
    expr: &'a SemExpr,
    width: u16,
    byte_index: u16,
    offset: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeCallTarget {
    Address(u16),
}

mod native_classify;
use native_classify::*;
mod native_emit;
mod native_materialize;
use native_materialize::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeForStep {
    Up(u16),
    Down(u16),
}

impl<'a, 'm> SemIrNativeEmitter<'a, 'm> {
    fn new(model: &'m SemIrReadModel<'a>) -> Self {
        Self {
            model,
            storage: HashMap::new(),
            machine_caret_values: HashMap::new(),
            emitter: NativeTrackedEmitter::with_origin(model.origin),
            routine_addresses: Vec::new(),
            routine_ranges: Vec::new(),
            routine_entries: HashMap::new(),
            routine_labels: HashMap::new(),
            runtime_helpers: RuntimeHelperTargets::default_for_target(RuntimeTarget::Cartridge),
            storage_symbols: Vec::new(),
            record_layouts: HashMap::new(),
            record_fields_by_id: HashMap::new(),
            source_ranges: Vec::new(),
            array_backings: Vec::new(),
            external_storage_cursor: None,
            main_run_address: None,
            last_routine_run_address: None,
            label_counter: 0,
            y_known_zero: false,
            current_return_width: None,
            current_routine: None,
            exit_labels: Vec::new(),
        }
    }

    fn emit(mut self) -> Result<CodegenOutput, String> {
        self.validate_items()?;
        self.emit_global_storage()?;
        self.ensure_routine_labels();
        self.apply_runtime_helper_sets();
        self.emit_routines()?;
        self.apply_symbol_sets()?;
        self.emit_rts();
        let skipped_ranges = self.bind_array_backings()?;

        let bytes = self.emitter.finish().map_err(|diagnostics| {
            diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.message)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        let run_address = self
            .main_run_address
            .or(self.last_routine_run_address)
            .unwrap_or(self.model.origin);
        let routine_addresses = self.routine_addresses;
        let routine_ranges = self.routine_ranges;
        let routine_signatures = self
            .model
            .routines
            .iter()
            .map(semir_native_routine_signature)
            .collect::<Vec<_>>();
        let mut storage_symbols = self.storage_symbols;
        storage_symbols.sort_by(|left, right| {
            native_symbol_scope_key(&left.scope)
                .cmp(&native_symbol_scope_key(&right.scope))
                .then_with(|| left.name.cmp(&right.name))
        });
        let source_ranges = self.source_ranges;
        let optimizations = Vec::new();
        let proofs = Vec::new();
        let proof_attempts = Vec::new();
        let map = CodegenMap {
            origin: self.model.origin,
            run_address,
            skipped_ranges: skipped_ranges.clone(),
            routine_addresses: routine_addresses.clone(),
            routine_ranges,
            routine_signatures,
            storage_symbols,
            source_ranges,
            routine_effects: Vec::new(),
            machine_blocks: Vec::new(),
            optimizations: optimizations.clone(),
            proofs: proofs.clone(),
            proof_attempts: proof_attempts.clone(),
        };

        Ok(CodegenOutput {
            bytes,
            origin: self.model.origin,
            run_address,
            skipped_ranges,
            routine_addresses,
            optimizations,
            proofs,
            proof_attempts,
            map,
        })
    }

    fn validate_items(&self) -> Result<(), String> {
        for module in &self.model.program.modules {
            for item in &module.items {
                match item {
                    SemItem::Define(_)
                    | SemItem::Set(_)
                    | SemItem::Declaration(_)
                    | SemItem::Routine(_) => {}
                    SemItem::Include(_) => {
                        return Err("INCLUDE items are not supported".to_string());
                    }
                    SemItem::Statement(_) => {
                        return Err("top-level statements are not supported".to_string());
                    }
                    SemItem::Unsupported { note, .. } => {
                        return Err(format!("unsupported SemIR item: {note}"));
                    }
                }
            }
        }
        Ok(())
    }

    fn bind_array_backings(&mut self) -> Result<Vec<SkippedRange>, String> {
        let mut position = self.emitter.position();
        let mut skipped_ranges = Vec::new();
        for backing in &self.array_backings {
            self.emitter
                .bind_label_at_position(backing.label.clone(), position, Span::new(0, 0))
                .map_err(|diagnostic| diagnostic.message)?;
            let start = self
                .model
                .origin
                .checked_add(position as u16)
                .ok_or_else(|| "native array backing address overflow".to_string())?;
            skipped_ranges.push(SkippedRange {
                start,
                len: backing.size,
            });
            position = position.saturating_add(usize::from(backing.size));
        }
        Ok(skipped_ranges)
    }

    fn emit_global_storage(&mut self) -> Result<(), String> {
        for module in &self.model.program.modules {
            for item in &module.items {
                match item {
                    SemItem::Set(set) => self.apply_storage_cursor_set(set),
                    SemItem::Declaration(declaration) => {
                        match declaration_group_kind(declaration) {
                            SemIrDeclarationGroupKind::Variables => {
                                self.emit_global_declaration(declaration)?;
                            }
                            SemIrDeclarationGroupKind::Type | SemIrDeclarationGroupKind::Record => {
                                self.record_native_record_layout(declaration)?;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn apply_storage_cursor_set(&mut self, set: &SemSet) {
        let Some(address) = self.constant_word(&set.address) else {
            return;
        };
        let Some(value) = self.constant_word(&set.value) else {
            return;
        };

        match address {
            0x000E if value <= 0x00FF => {
                self.external_storage_cursor = Some(value);
            }
            0x000E => {
                self.external_storage_cursor = None;
            }
            0x000F if value == 0 => {}
            0x000F => {
                self.external_storage_cursor = None;
            }
            _ => {}
        }
    }

    fn record_native_record_layout(&mut self, declaration: &SemDeclaration) -> Result<(), String> {
        let record_type = match &declaration.storage {
            SemDeclarationStorage::Type { record_type, .. }
            | SemDeclarationStorage::Record { record_type, .. } => record_type,
            _ => return Ok(()),
        };

        let mut native_fields = HashMap::new();
        for field in &record_type.fields {
            let width = self
                .native_type_width(&field.ty)
                .ok_or_else(|| format!("unsupported record field type `{}`", field.name))?;
            native_fields.insert(
                normalize_name(&field.name),
                NativeRecordField {
                    offset: field.offset,
                    width,
                },
            );
            if let Some(id) = field.id {
                self.record_fields_by_id.insert(
                    id,
                    NativeRecordField {
                        offset: field.offset,
                        width,
                    },
                );
            }
        }
        self.record_layouts.insert(
            normalize_name(&declaration.symbol.name),
            NativeRecordLayout {
                record_type: record_type.clone(),
                size: record_type.size,
                fields: native_fields,
            },
        );
        Ok(())
    }

    fn emit_global_declaration(&mut self, declaration: &SemDeclaration) -> Result<(), String> {
        let source_start = self.current_address()?;
        let (address, slot) = match &declaration.storage {
            SemDeclarationStorage::Scalar => {
                let width = self.native_sem_type_width(&declaration.ty).ok_or_else(|| {
                    format!("unsupported scalar type `{}`", declaration.symbol.name)
                })?;
                self.emit_scalar_storage(
                    declaration,
                    width,
                    self.native_sem_pointee_width(&declaration.ty),
                    self.native_sem_record_name(&declaration.ty),
                )?
            }
            SemDeclarationStorage::Array { array_type, .. } => {
                let array = self.native_array_facts(array_type, &declaration.symbol.name)?;
                self.emit_array_storage(
                    declaration,
                    array.element_width,
                    array.declared_len,
                    array.record,
                )?
            }
            SemDeclarationStorage::Type { .. } | SemDeclarationStorage::Record { .. } => {
                return Err("TYPE and RECORD declarations are not supported".to_string());
            }
        };
        debug_assert_eq!(address, slot.address);
        self.storage.insert(declaration.symbol.id, slot.clone());
        self.record_storage_symbol(
            declaration.symbol.name.clone(),
            CodegenSymbolScope::Global,
            CodegenSymbolKind::Storage,
            slot,
        );
        let source_end = self.current_address()?;
        self.record_source_range(
            CodegenSourceRangeKind::Declaration,
            Some(declaration.symbol.name.clone()),
            declaration.span,
            source_start,
            source_end,
        );
        if declaration.initializer.is_some() && source_end > source_start {
            self.record_source_range(
                CodegenSourceRangeKind::StorageInitializer,
                Some(declaration.symbol.name.clone()),
                declaration.span,
                source_start,
                source_end,
            );
        }
        Ok(())
    }

    fn emit_scalar_storage(
        &mut self,
        declaration: &SemDeclaration,
        width: u16,
        pointee_width: Option<u16>,
        record: Option<String>,
    ) -> Result<(u16, NativeStorageSlot), String> {
        if let Some(initializer) = &declaration.initializer {
            if let Some(bytes) = raw_initializer_bytes(initializer, width)? {
                let address = self.current_address()?;
                self.emit_raw_bytes(bytes);
                return Ok((
                    address,
                    NativeStorageSlot {
                        address,
                        width,
                        array: None,
                        pointee_width,
                        record: record.clone(),
                    },
                ));
            }
            if let Some(address) = self.constant_word(initializer) {
                return Ok((
                    address,
                    NativeStorageSlot {
                        address,
                        width,
                        array: None,
                        pointee_width,
                        record: record.clone(),
                    },
                ));
            }
            return Err(format!(
                "unsupported initializer for `{}`",
                declaration.symbol.name
            ));
        }

        if let Some(address) = self.external_storage_cursor {
            self.external_storage_cursor = Some(address.wrapping_add(width));
            return Ok((
                address,
                NativeStorageSlot {
                    address,
                    width,
                    array: None,
                    pointee_width,
                    record,
                },
            ));
        }

        let address = self.current_address()?;
        self.emit_raw_zeroes(width);
        Ok((
            address,
            NativeStorageSlot {
                address,
                width,
                array: None,
                pointee_width,
                record,
            },
        ))
    }

    fn emit_array_storage(
        &mut self,
        declaration: &SemDeclaration,
        element_width: u16,
        explicit_len: Option<u16>,
        record: Option<String>,
    ) -> Result<(u16, NativeStorageSlot), String> {
        let bytes = if let Some(initializer) = &declaration.initializer {
            if let Some(text) = string_initializer_bytes(initializer, explicit_len)? {
                text
            } else if let Some(values) = raw_initializer_values(initializer)? {
                numeric_storage_bytes(&values, element_width, explicit_len)
            } else if let Some(label) = self.symbolic_array_address_initializer(initializer) {
                let slot_address = self.current_address()?;
                self.emit_raw_u16_label(label.clone(), declaration.span);
                if explicit_len.is_some() {
                    self.emit_raw_u16_label(label, declaration.span);
                }
                return Ok((
                    slot_address,
                    NativeStorageSlot {
                        address: slot_address,
                        width: 2,
                        array: Some(NativeArrayStorage {
                            element_width,
                            len: explicit_len.unwrap_or(0),
                            storage: CodegenArrayStorage::Pointer,
                        }),
                        pointee_width: None,
                        record: record.clone(),
                    },
                ));
            } else if let Some(address) = self.absolute_array_address_initializer(initializer) {
                self.record_machine_caret_value(declaration, address);
                if explicit_len.is_none() {
                    let slot_address = self.current_address()?;
                    self.emit_raw_u16_le(address);
                    return Ok((
                        slot_address,
                        NativeStorageSlot {
                            address: slot_address,
                            width: 2,
                            array: Some(NativeArrayStorage {
                                element_width,
                                len: 0,
                                storage: CodegenArrayStorage::Pointer,
                            }),
                            pointee_width: None,
                            record: record.clone(),
                        },
                    ));
                }
                if explicit_len.is_some_and(|len| len > 0x00FF) {
                    let descriptor_address = self.current_address()?;
                    self.emit_raw_bytes(fixed_array_pointer_storage(address));
                    return Ok((
                        descriptor_address,
                        NativeStorageSlot {
                            address: descriptor_address,
                            width: 2,
                            array: Some(NativeArrayStorage {
                                element_width,
                                len: explicit_len.unwrap_or(0),
                                storage: CodegenArrayStorage::Pointer,
                            }),
                            pointee_width: None,
                            record: record.clone(),
                        },
                    ));
                }
                return Ok((
                    address,
                    NativeStorageSlot {
                        address,
                        width: element_width,
                        array: Some(NativeArrayStorage {
                            element_width,
                            len: explicit_len.unwrap_or(0),
                            storage: CodegenArrayStorage::Inline,
                        }),
                        pointee_width: None,
                        record: record.clone(),
                    },
                ));
            } else if let Some(address) = self.constant_word(initializer) {
                self.record_machine_caret_value(declaration, address);
                let slot_address = self.current_address()?;
                if explicit_len.is_some() {
                    self.emit_raw_bytes(fixed_array_pointer_storage(address));
                } else {
                    self.emit_raw_u16_le(address);
                }
                return Ok((
                    slot_address,
                    NativeStorageSlot {
                        address: slot_address,
                        width: 2,
                        array: Some(NativeArrayStorage {
                            element_width,
                            len: explicit_len.unwrap_or(0),
                            storage: CodegenArrayStorage::Pointer,
                        }),
                        pointee_width: None,
                        record: record.clone(),
                    },
                ));
            } else {
                return Err(format!(
                    "unsupported array initializer for `{}`",
                    declaration.symbol.name
                ));
            }
        } else {
            let Some(len) = explicit_len else {
                let address = self.current_address()?;
                self.emit_raw_zeroes(2);
                return Ok((
                    address,
                    NativeStorageSlot {
                        address,
                        width: 2,
                        array: Some(NativeArrayStorage {
                            element_width,
                            len: 0,
                            storage: CodegenArrayStorage::Pointer,
                        }),
                        pointee_width: None,
                        record: record.clone(),
                    },
                ));
            };
            let byte_len = len
                .checked_mul(element_width)
                .ok_or_else(|| "array byte size overflow".to_string())?;
            if element_width == 1 && len <= 0x0100 {
                native_sized_byte_array_storage_bytes(byte_len, len)
            } else {
                let address = self.current_address()?;
                let label = self.next_label(&format!("array_{}", declaration.symbol.name));
                self.emit_raw_u16_label(label.clone(), declaration.span);
                self.emit_raw_u8((byte_len & 0x00FF) as u8);
                self.emit_raw_u8((byte_len >> 8) as u8);
                self.array_backings.push(NativeArrayBacking {
                    label,
                    size: byte_len,
                });
                return Ok((
                    address,
                    NativeStorageSlot {
                        address,
                        width: element_width,
                        array: Some(NativeArrayStorage {
                            element_width,
                            len,
                            storage: CodegenArrayStorage::Descriptor,
                        }),
                        pointee_width: None,
                        record: record.clone(),
                    },
                ));
            }
        };

        let address = self.current_address()?;
        self.emit_raw_bytes(bytes.iter().copied());
        let len = array_len_from_bytes(&bytes, element_width);
        Ok((
            address,
            NativeStorageSlot {
                address,
                width: element_width,
                array: Some(NativeArrayStorage {
                    element_width,
                    len,
                    storage: CodegenArrayStorage::Inline,
                }),
                pointee_width: None,
                record,
            },
        ))
    }

    fn record_machine_caret_value(&mut self, declaration: &SemDeclaration, value: u16) {
        self.machine_caret_values
            .insert(declaration.symbol.id, value);
    }

    fn symbolic_array_address_initializer(&mut self, expr: &SemExpr) -> Option<String> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.symbolic_array_address_initializer(expr),
            SemExprKind::Symbol(symbol)
                if matches!(symbol.class, SymbolClass::Proc | SymbolClass::Func) =>
            {
                Some(self.routine_label(symbol))
            }
            _ => None,
        }
    }

    fn routine_label(&mut self, symbol: &SemSymbolRef) -> String {
        if let Some(label) = self.routine_labels.get(&symbol.id) {
            return label.clone();
        }
        let label = self.next_label(&format!("routine_{}", normalize_name(&symbol.name)));
        self.routine_labels.insert(symbol.id, label.clone());
        label
    }

    fn absolute_array_address_initializer(&self, expr: &SemExpr) -> Option<u16> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.absolute_array_address_initializer(expr),
            SemExprKind::Literal(SemLiteral::Number(_) | SemLiteral::Char(_)) => {
                self.constant_word(expr)
            }
            SemExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Neg,
                ..
            } => self.constant_word(expr),
            _ => None,
        }
    }

    fn native_sem_type_width(&self, ty: &SemType) -> Option<u16> {
        self.native_type_width(&ty.value)
    }

    fn native_type_width(&self, ty: &ValueType) -> Option<u16> {
        ty.value_width_bytes().or_else(|| {
            let name = ty.as_record_base_name()?;
            self.record_layouts
                .get(&normalize_name(name))
                .map(|layout| layout.size)
        })
    }

    fn native_sem_pointee_width(&self, ty: &SemType) -> Option<u16> {
        ty.value
            .as_pointer()
            .and_then(|pointer| self.native_type_width(&pointer.pointee))
    }

    fn native_sem_record_name(&self, ty: &SemType) -> Option<String> {
        self.native_record_name_for_type(&ty.value)
    }

    fn native_record_name_for_type(&self, ty: &ValueType) -> Option<String> {
        let name = normalize_name(ty.as_record_base_name()?);
        if self.record_layouts.contains_key(&name) {
            Some(name)
        } else {
            None
        }
    }

    fn native_array_facts(
        &self,
        array_type: &ArrayType,
        symbol_name: &str,
    ) -> Result<NativeArrayFacts, String> {
        let element_width = self
            .native_type_width(&array_type.element)
            .ok_or_else(|| format!("unsupported array element type `{symbol_name}`"))?;
        Ok(NativeArrayFacts {
            element_width,
            declared_len: array_type.length,
            record: self.native_record_name_for_type(&array_type.element),
        })
    }

    fn emit_routines(&mut self) -> Result<(), String> {
        self.ensure_routine_labels();
        for routine in &self.model.routines {
            self.emit_routine(routine.routine)?;
        }
        Ok(())
    }

    fn ensure_routine_labels(&mut self) {
        let routine_symbols = self
            .model
            .routines
            .iter()
            .map(|routine| {
                (
                    routine.routine.symbol.id,
                    routine.routine.symbol.name.clone(),
                    routine.routine.span,
                )
            })
            .collect::<Vec<_>>();
        for (id, name, _) in routine_symbols {
            if !self.routine_labels.contains_key(&id) {
                let label = self.next_label(&format!("routine_{}", normalize_name(&name)));
                self.routine_labels.insert(id, label);
            }
        }
    }

    fn emit_routine(&mut self, routine: &SemRoutine) -> Result<(), String> {
        let current_location_routine = routine
            .system_address
            .as_ref()
            .is_some_and(is_current_location_expr);
        if let Some(system_address) = &routine.system_address
            && !current_location_routine
        {
            let Some(address) = self.constant_word(system_address) else {
                return Err(format!(
                    "routine `{}` system address is not constant",
                    routine.symbol.name
                ));
            };
            self.routine_addresses.push(RoutineAddress {
                name: routine.symbol.name.clone(),
                address,
            });
            self.routine_entries.insert(routine.symbol.id, address);
            return Ok(());
        }
        let routine_range_start = self.current_address()?;
        self.emit_param_storage(routine)?;
        self.emit_local_storage(routine)?;
        let routine_start = self.current_address()?;
        if let Some(label) = self.routine_labels.get(&routine.symbol.id).cloned() {
            self.bind_label(&label, routine.span)?;
        }
        self.y_known_zero = false;
        self.routine_addresses.push(RoutineAddress {
            name: routine.symbol.name.clone(),
            address: routine_start,
        });
        self.routine_entries
            .insert(routine.symbol.id, routine_start);
        self.last_routine_run_address = Some(routine_start);
        if routine.symbol.name.eq_ignore_ascii_case("main") {
            self.main_run_address = Some(routine_start);
        }

        if !current_location_routine {
            let body_start = routine_start
                .checked_add(3)
                .ok_or_else(|| "routine entry address overflow".to_string())?;
            self.emit_jmp_addr(body_start);
            self.emit_param_prologue(routine)?;
        }
        self.y_known_zero = false;
        let previous_return_width = self.current_return_width;
        let previous_routine = self.current_routine;
        self.current_return_width = routine_return_width(routine);
        self.current_routine = Some(routine.symbol.id);
        for stmt in &routine.body {
            self.emit_statement(stmt)?;
        }
        self.current_return_width = previous_return_width;
        self.current_routine = previous_routine;
        let routine_end = self.current_address()?;
        self.routine_ranges.push(RoutineRange {
            name: routine.symbol.name.clone(),
            start: routine_range_start,
            end: routine_end,
        });
        self.record_source_range(
            CodegenSourceRangeKind::Routine,
            Some(routine.symbol.name.clone()),
            routine.span,
            routine_range_start,
            routine_end,
        );
        Ok(())
    }

    fn emit_param_storage(&mut self, routine: &SemRoutine) -> Result<(), String> {
        for param in &routine.params {
            let (width, array, record) = match param.storage {
                SemParamStorage::Value => {
                    let width = self.native_sem_type_width(&param.ty).ok_or_else(|| {
                        format!("unsupported parameter type `{}`", param.symbol.name)
                    })?;
                    (width, None, self.native_sem_record_name(&param.ty))
                }
                SemParamStorage::Array => {
                    let array_type = param.array_type.as_ref().ok_or_else(|| {
                        format!(
                            "array parameter `{}` lacks semantic array type",
                            param.symbol.name
                        )
                    })?;
                    let facts = self.native_array_facts(array_type, &param.symbol.name)?;
                    (
                        2,
                        Some(NativeArrayStorage {
                            element_width: facts.element_width,
                            len: facts.declared_len.unwrap_or(0),
                            storage: CodegenArrayStorage::Pointer,
                        }),
                        facts.record,
                    )
                }
            };
            let address = self.current_address()?;
            self.emit_raw_zeroes(width);
            let slot = NativeStorageSlot {
                address,
                width,
                array,
                pointee_width: self.native_sem_pointee_width(&param.ty),
                record,
            };
            self.storage.insert(param.symbol.id, slot.clone());
            self.record_storage_symbol(
                param.symbol.name.clone(),
                CodegenSymbolScope::Routine(routine.symbol.name.clone()),
                CodegenSymbolKind::Parameter,
                slot,
            );
            self.record_source_range(
                CodegenSourceRangeKind::StorageInitializer,
                Some(format!("{} storage", routine.symbol.name)),
                param.span,
                address,
                address + width,
            );
        }
        Ok(())
    }

    fn emit_local_storage(&mut self, routine: &SemRoutine) -> Result<(), String> {
        for local in &routine.locals {
            let source_start = self.current_address()?;
            let (address, slot) = match &local.storage {
                SemDeclarationStorage::Scalar => {
                    let Some(width) = self.native_sem_type_width(&local.ty) else {
                        if native_define_artifact_local(local) {
                            continue;
                        }
                        return Err(format!("unsupported local type `{}`", local.symbol.name));
                    };
                    self.emit_scalar_storage(
                        local,
                        width,
                        self.native_sem_pointee_width(&local.ty),
                        self.native_sem_record_name(&local.ty),
                    )?
                }
                SemDeclarationStorage::Array { array_type, .. } => {
                    let array = self.native_array_facts(array_type, &local.symbol.name)?;
                    self.emit_array_storage(
                        local,
                        array.element_width,
                        array.declared_len,
                        array.record,
                    )?
                }
                SemDeclarationStorage::Type { .. } | SemDeclarationStorage::Record { .. } => {
                    self.record_native_record_layout(local)?;
                    continue;
                }
            };
            debug_assert_eq!(address, slot.address);
            self.storage.insert(local.symbol.id, slot.clone());
            self.record_storage_symbol(
                local.symbol.name.clone(),
                CodegenSymbolScope::Routine(routine.symbol.name.clone()),
                CodegenSymbolKind::Local,
                slot,
            );
            let source_end = self.current_address()?;
            self.record_source_range(
                CodegenSourceRangeKind::Declaration,
                Some(local.symbol.name.clone()),
                local.span,
                source_start,
                source_end,
            );
            if local.initializer.is_some() && source_end > source_start {
                self.record_source_range(
                    CodegenSourceRangeKind::StorageInitializer,
                    Some(local.symbol.name.clone()),
                    local.span,
                    source_start,
                    source_end,
                );
            }
        }
        Ok(())
    }

    fn emit_param_prologue(&mut self, routine: &SemRoutine) -> Result<(), String> {
        let total_width = self.routine_param_total_width(routine)?;
        if total_width > 2 {
            let first_param = routine.params.first().ok_or_else(|| {
                format!("routine `{}` has no first parameter", routine.symbol.name)
            })?;
            let base = self
                .storage
                .get(&first_param.symbol.id)
                .map(|slot| slot.address)
                .ok_or_else(|| {
                    format!("symbol `{}` has no native storage", first_param.symbol.name)
                })?;
            self.emit_jsr_runtime_helper(
                self.runtime_helpers.target(RuntimeHelperSlot::SArgs),
                routine.span,
            );
            self.emit_raw_u8((base & 0x00FF) as u8);
            self.emit_raw_u8((base >> 8) as u8);
            self.emit_raw_u8((total_width - 1) as u8);
            return Ok(());
        }
        for (index, param) in routine.params.iter().enumerate().rev() {
            let storage_slot =
                self.storage.get(&param.symbol.id).cloned().ok_or_else(|| {
                    format!("symbol `{}` has no native storage", param.symbol.name)
                })?;
            let slot = NativeResolvedSlot {
                address: storage_slot.address,
                width: storage_slot.width,
                pointee_width: storage_slot.pointee_width,
                record: storage_slot.record.clone(),
            };
            match (index, slot.width) {
                (0, 1) => self.emit_sta_addr(slot.address),
                (1, 1) => self.emit_stx_addr(slot.address),
                (0, 2) => {
                    self.emit_stx_addr(slot.address + 1);
                    self.emit_sta_addr(slot.address);
                }
                _ => {
                    return Err(format!(
                        "routine `{}` only supports byte parameters or first word parameter",
                        routine.symbol.name
                    ));
                }
            }
        }
        Ok(())
    }

    fn routine_param_total_width(&self, routine: &SemRoutine) -> Result<u16, String> {
        routine.params.iter().try_fold(0u16, |total, param| {
            let width = match param.storage {
                SemParamStorage::Value => self
                    .native_sem_type_width(&param.ty)
                    .ok_or_else(|| format!("unsupported parameter type `{}`", param.symbol.name))?,
                SemParamStorage::Array => 2,
            };
            total
                .checked_add(width)
                .ok_or_else(|| "parameter byte count overflow".to_string())
        })
    }

    fn emit_statement(&mut self, stmt: &SemStmt) -> Result<(), String> {
        let start = self.current_address()?;
        let result = self.emit_statement_inner(stmt).map_err(|err| {
            let span = stmt_span(stmt);
            format!(
                "{} {}..{}: {err}",
                stmt_kind_name(stmt),
                span.start,
                span.end
            )
        });
        if result.is_ok() {
            let end = self.current_address()?;
            self.record_source_range(
                CodegenSourceRangeKind::Statement,
                Some(stmt_kind_name(stmt).to_string()),
                stmt_span(stmt),
                start,
                end,
            );
        }
        result
    }

    fn emit_statement_inner(&mut self, stmt: &SemStmt) -> Result<(), String> {
        match stmt {
            SemStmt::Define(_) => Ok(()),
            SemStmt::Assign { target, value, .. } => self.emit_assignment(target, value),
            SemStmt::CompoundAssign {
                target, op, value, ..
            } => self.emit_compound_assignment(target, *op, value),
            SemStmt::Return { value: None, .. } => {
                self.emit_rts();
                Ok(())
            }
            SemStmt::Return {
                value: Some(value), ..
            } => self.emit_return_value(value),
            SemStmt::Call { call, .. } => self.emit_call(call),
            SemStmt::MachineBlock { items, span, .. } => self.emit_machine_block(items, *span),
            SemStmt::If {
                branches,
                else_body,
                ..
            } => self.emit_if(branches, else_body),
            SemStmt::While {
                condition, body, ..
            } => self.emit_while(condition, body),
            SemStmt::DoUntil {
                body,
                condition,
                span,
            } => self.emit_do_until(body, condition.as_ref(), *span),
            SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                span,
            } => self.emit_for(target, start, end, step.as_ref(), body, *span),
            SemStmt::Exit { span } => self.emit_exit(*span),
            other => Err(format!(
                "statement `{}` is not supported",
                stmt_kind_name(other)
            )),
        }
    }

    fn record_storage_symbol(
        &mut self,
        name: String,
        scope: CodegenSymbolScope,
        kind: CodegenSymbolKind,
        slot: NativeStorageSlot,
    ) {
        self.storage_symbols.push(CodegenStorageSymbol {
            name,
            scope,
            kind,
            address: slot.address,
            size: native_slot_size(&slot),
            address_space: CodegenAddressSpace::Absolute,
            pointee_size: slot.pointee_width,
            array: slot.array.map(|array| array.storage),
            signed: false,
        });
    }

    fn record_source_range(
        &mut self,
        kind: CodegenSourceRangeKind,
        name: Option<String>,
        source_span: Span,
        start: u16,
        end: u16,
    ) {
        self.source_ranges.push(CodegenSourceRange {
            kind,
            name,
            source_span,
            start,
            end,
        });
    }

    fn apply_symbol_sets(&mut self) -> Result<(), String> {
        for module in &self.model.program.modules {
            for item in &module.items {
                let SemItem::Set(set) = item else {
                    continue;
                };
                let Some(symbol) = self.set_address_symbol(&set.address) else {
                    continue;
                };
                let Some(slot) = self.storage.get(&symbol.id).cloned() else {
                    continue;
                };
                let Some(value) = self.symbol_set_value(&set.value) else {
                    continue;
                };
                let width = native_slot_size(&slot);
                if width == 0 {
                    continue;
                }
                self.emitter
                    .patch_absolute_bytes(slot.address, value, width.min(2));
            }
        }
        Ok(())
    }

    fn symbol_set_value(&self, expr: &SemExpr) -> Option<u16> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.symbol_set_value(expr),
            SemExprKind::CurrentLocation => self.current_high_water_address().ok(),
            _ => self.constant_word(expr),
        }
    }

    fn current_high_water_address(&self) -> Result<u16, String> {
        let current = self.current_address()?;
        if self.array_backings.is_empty() {
            return Ok(current);
        }

        let after_final_rts = current
            .checked_add(1)
            .ok_or_else(|| "native current-location high-water overflow".to_string())?;
        self.array_backings
            .iter()
            .try_fold(after_final_rts, |address, backing| {
                address
                    .checked_add(backing.size)
                    .ok_or_else(|| "native current-location high-water overflow".to_string())
            })
    }

    fn apply_runtime_helper_sets(&mut self) {
        for module in &self.model.program.modules {
            for item in &module.items {
                let SemItem::Set(set) = item else {
                    continue;
                };
                let Some(address) = self.constant_word(&set.address) else {
                    continue;
                };
                let Some(value) = self.runtime_helper_set_target(&set.value) else {
                    continue;
                };
                self.runtime_helpers.apply_set(address, value);
            }
        }
    }

    fn runtime_helper_set_target(&self, expr: &SemExpr) -> Option<RuntimeHelperTarget> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.runtime_helper_set_target(expr),
            SemExprKind::Symbol(symbol) | SemExprKind::AddressOfSymbol(symbol) => {
                self.routine_helper_target(symbol).or_else(|| {
                    self.constant_word(expr)
                        .map(|value| Absolute::new(value).into())
                })
            }
            _ => self
                .constant_word(expr)
                .map(|value| Absolute::new(value).into()),
        }
    }

    fn routine_helper_target(&self, symbol: &SemSymbolRef) -> Option<RuntimeHelperTarget> {
        if let Some(routine) = self
            .model
            .routines
            .iter()
            .find(|routine| routine.routine.symbol.id == symbol.id)
            && let Some(system_address) = &routine.routine.system_address
            && !is_current_location_expr(system_address)
            && let Some(address) = self.constant_word(system_address)
        {
            return Some(RuntimeHelperTarget::Absolute(Absolute::new(address)));
        }
        self.routine_labels
            .get(&symbol.id)
            .cloned()
            .map(RuntimeHelperTarget::Label)
    }

    fn set_address_symbol<'s>(&self, expr: &'s SemExpr) -> Option<&'s SemSymbolRef> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.set_address_symbol(expr),
            SemExprKind::Symbol(symbol) => Some(symbol),
            SemExprKind::LValue(lvalue) => self.lvalue_symbol(lvalue),
            SemExprKind::ArrayDecay(decay) => self.lvalue_symbol(&decay.array),
            _ => None,
        }
    }

    fn lvalue_symbol<'s>(&self, lvalue: &'s SemLValue) -> Option<&'s SemSymbolRef> {
        match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => Some(symbol),
            _ => None,
        }
    }

    fn constant_word(&self, expr: &SemExpr) -> Option<u16> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.constant_word(expr),
            SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
            SemExprKind::Literal(SemLiteral::Char(ch)) => {
                let codepoint = u32::from(*ch);
                u16::try_from(codepoint).ok()
            }
            SemExprKind::CurrentLocation => self.current_address().ok(),
            SemExprKind::Symbol(symbol) => self
                .storage
                .get(&symbol.id)
                .map(|slot| slot.address)
                .or_else(|| self.routine_entries.get(&symbol.id).copied())
                .or_else(|| self.numeric_define(&symbol.name)),
            SemExprKind::AddressOfSymbol(symbol) => self
                .storage
                .get(&symbol.id)
                .map(|slot| slot.address)
                .or_else(|| self.routine_entries.get(&symbol.id).copied()),
            SemExprKind::LValue(lvalue) => self.lvalue_base_address(lvalue),
            SemExprKind::ArrayDecay(decay) => self.lvalue_base_address(&decay.array),
            SemExprKind::AddressOf(lvalue) => self.lvalue_address(lvalue).ok(),
            SemExprKind::ImplicitAddressOf(address) => self.lvalue_address(&address.place).ok(),
            SemExprKind::Unary { op, expr } => {
                let value = self.constant_word(expr)?;
                match op {
                    UnaryOp::Plus => Some(value),
                    UnaryOp::Neg => Some(0u16.wrapping_sub(value)),
                    UnaryOp::AddressOf | UnaryOp::Deref => None,
                }
            }
            SemExprKind::Binary { op, left, right } => {
                let left = self.constant_word(left)?;
                let right = self.constant_word(right)?;
                match op {
                    BinaryOp::Add => Some(left.wrapping_add(right)),
                    BinaryOp::Sub => Some(left.wrapping_sub(right)),
                    BinaryOp::And => Some(left & right),
                    BinaryOp::Or => Some(left | right),
                    BinaryOp::Xor => Some(left ^ right),
                    BinaryOp::Lsh => Some(left.wrapping_shl(u32::from(right))),
                    BinaryOp::Rsh => Some(left.wrapping_shr(u32::from(right))),
                    BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Eq
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

    fn constant_return_word(&self, expr: &SemExpr) -> Option<u16> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.constant_return_word(expr),
            SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
            SemExprKind::Literal(SemLiteral::Char(ch)) => {
                let codepoint = u32::from(*ch);
                u16::try_from(codepoint).ok()
            }
            SemExprKind::Symbol(symbol) => self.numeric_define(&symbol.name),
            SemExprKind::Unary { op, expr } => {
                let value = self.constant_return_word(expr)?;
                match op {
                    UnaryOp::Plus => Some(value),
                    UnaryOp::Neg => Some(0u16.wrapping_sub(value)),
                    UnaryOp::AddressOf | UnaryOp::Deref => None,
                }
            }
            SemExprKind::Binary { op, left, right } => {
                let left = self.constant_return_word(left)?;
                let right = self.constant_return_word(right)?;
                match op {
                    BinaryOp::Add => Some(left.wrapping_add(right)),
                    BinaryOp::Sub => Some(left.wrapping_sub(right)),
                    BinaryOp::And => Some(left & right),
                    BinaryOp::Or => Some(left | right),
                    BinaryOp::Xor => Some(left ^ right),
                    BinaryOp::Lsh => Some(left.wrapping_shl(u32::from(right))),
                    BinaryOp::Rsh => Some(left.wrapping_shr(u32::from(right))),
                    BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Eq
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

    fn emit_assignment(&mut self, target: &SemLValue, value: &SemExpr) -> Result<(), String> {
        if matches!(target.kind, SemLValueKind::Field { .. }) {
            return self.emit_record_field_assignment(target, value);
        }
        if self.classifier().pointer_deref_lvalue(target).is_some() {
            return self.emit_deref_assignment(target, value);
        }
        if self.emit_pointer_index_assignment(target, value)? {
            return Ok(());
        }
        if self.emit_dynamic_array_assignment(target, value)? {
            return Ok(());
        }
        let target_slot = self.classifier().required_lvalue_slot(target)?;

        if self.materialize_value_to_target(value, target_slot.clone())? {
            return Ok(());
        }
        if self.emit_expr_to_target(value, target_slot.clone())? {
            return Ok(());
        }

        if self.emit_deref_expr_to_target(value, target_slot.clone())? {
            return Ok(());
        }
        if self.emit_deref_expr_to_a(value)? {
            self.emit_sta_addr(target_slot.address);
            return Ok(());
        }
        if self.emit_pointer_index_expr_to_a(value)? {
            self.emit_sta_addr(target_slot.address);
            return Ok(());
        }
        if self.emit_array_index_expr_to_a(value)? {
            self.emit_sta_addr(target_slot.address);
            return Ok(());
        }
        if self.emit_record_field_expr_to_target(value, target_slot.clone())? {
            return Ok(());
        }

        let source_slot = self
            .classifier()
            .required_addressable_slot(value)
            .map_err(|reason| {
                format!(
                    "{reason} (assigning {} to {})",
                    native_expr_debug_name(value),
                    native_lvalue_debug_name(target)
                )
            })?;
        if source_slot.width != target_slot.width {
            return Err("assignment width mismatch is not supported".to_string());
        }
        if !self.materialize_slot_to_target(source_slot, target_slot)? {
            return Err("only byte and word assignments are supported".to_string());
        }
        Ok(())
    }

    fn emit_dynamic_array_assignment(
        &mut self,
        target: &SemLValue,
        value: &SemExpr,
    ) -> Result<bool, String> {
        let Some(indexed) = self.classifier().array_index_lvalue(target)? else {
            return Ok(false);
        };
        if !matches!(indexed.element_width, 1 | 2) {
            return Err("only byte and word dynamic array assignments are supported".to_string());
        }
        if self.expr_contains_routine_call(indexed.index) {
            return self.emit_dynamic_array_assignment_with_staged_index(indexed, value);
        }

        self.emit_array_assignment_value_to_args(value, indexed.element_width)?;
        match indexed.storage {
            CodegenArrayStorage::Inline => {
                self.materialize_args_to_inline_array_element(indexed)?;
            }
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                let element_width = self.materialize_pointer_backed_array_index_address(
                    indexed,
                    NativeAddressDestination::ArrayAddr,
                )?;
                self.materialize_args_to_array_addr_element(element_width)?;
            }
        }
        Ok(true)
    }

    fn emit_dynamic_array_assignment_with_staged_index(
        &mut self,
        indexed: NativeArrayIndexAccess<'_>,
        value: &SemExpr,
    ) -> Result<bool, String> {
        self.emit_byte_expr_to_a(indexed.index)?;
        self.emit_pha();
        self.emit_array_assignment_value_to_args(value, indexed.element_width)?;
        self.emit_pla();

        match indexed.storage {
            CodegenArrayStorage::Inline => {
                if indexed.element_width == 2 {
                    self.emit_asl_a();
                }
                self.emit_tax();
                self.emit_array_assignment_args_to_inline_indexed(
                    indexed.slot.address,
                    indexed.element_width,
                )?;
            }
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                self.materialize_pointer_scaled_index_a_to_array_addr(
                    indexed.slot.address,
                    indexed.element_width,
                )?;
                self.materialize_args_to_array_addr_element(indexed.element_width)?;
            }
        }
        Ok(true)
    }

    fn emit_array_assignment_value_to_args(
        &mut self,
        value: &SemExpr,
        width: u16,
    ) -> Result<(), String> {
        match width {
            1 => {
                if !self.materialize_value_to_target(value, native_args_slot(1))? {
                    self.emit_byte_expr_to_a(value)?;
                    self.emit_sta_args(0);
                }
            }
            2 => {
                if !self.materialize_value_to_target(value, native_args_slot(2))?
                    && !self.emit_expr_to_target(value, native_args_slot(2))?
                {
                    return Err("only word array assignment values are supported".to_string());
                }
            }
            _ => return Err("only byte and word array assignment values are supported".to_string()),
        }
        Ok(())
    }

    fn emit_record_field_assignment(
        &mut self,
        target: &SemLValue,
        value: &SemExpr,
    ) -> Result<(), String> {
        let Some(access) = self.classifier().record_field_lvalue(target) else {
            return Err("expected record field".to_string());
        };
        let width =
            self.materialize_record_field_address(access, NativeAddressDestination::ArrayAddr)?;
        match width {
            1 | 2 => {
                if !self.materialize_value_to_array_addr_element(value, width)? {
                    return Err("only byte and word record field values are supported".to_string());
                }
            }
            _ => return Err("only byte and word record field stores are supported".to_string()),
        }
        Ok(())
    }

    fn emit_pointer_index_assignment(
        &mut self,
        target: &SemLValue,
        value: &SemExpr,
    ) -> Result<bool, String> {
        let Some(indexed) = self.classifier().pointer_index_lvalue(target)? else {
            return Ok(false);
        };
        let pointee_width = indexed.element_width;
        if pointee_width != 1 && pointee_width != 2 {
            return Err("only byte and word pointer indexed assignments are supported".to_string());
        }
        self.materialize_pointer_index_address(indexed, NativeAddressDestination::ArrayAddr)?;
        if !self.materialize_value_to_array_addr_element(value, pointee_width)? {
            return Err("only byte and word pointer indexed values are supported".to_string());
        }
        Ok(true)
    }

    fn emit_record_field_expr_to_target(
        &mut self,
        value: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let Some(access) = self.classifier().record_field_expr(value) else {
            return Ok(false);
        };
        if self.classifier().addressable_slot(value)?.is_some() {
            return Ok(false);
        }
        let width = access.width;
        if width != target.width {
            return Err("record field assignment width mismatch".to_string());
        }
        self.materialize_record_field_address(access, NativeAddressDestination::ArrayAddr)?;
        match self.materialize_array_addr_element_to_target(target)? {
            true => {}
            _ => return Err("only byte and word record field reads are supported".to_string()),
        }
        Ok(true)
    }

    fn emit_record_field_expr_to_a(&mut self, value: &SemExpr) -> Result<bool, String> {
        let Some(access) = self.classifier().record_field_expr(value) else {
            return Ok(false);
        };
        if access.width != 1 {
            return Ok(false);
        }
        self.materialize_record_field_address(access, NativeAddressDestination::ArrayAddr)?;
        self.materialize_array_addr_element_to_a();
        Ok(true)
    }

    fn native_record_field_layout(
        &self,
        base: &NativeResolvedSlot,
        field: &SemFieldRef,
    ) -> Result<NativeRecordField, String> {
        if let Some(offset) = field.offset
            && let Some(width) = self.native_type_width(&field.ty)
        {
            return Ok(NativeRecordField { offset, width });
        }

        if let Some(id) = field.id {
            return self
                .record_fields_by_id
                .get(&id)
                .copied()
                .ok_or_else(|| format!("record field id {:?} is not known", id));
        }

        let record = base
            .record
            .as_deref()
            .ok_or_else(|| "record field base must have record metadata".to_string())?;
        self.record_layouts
            .get(record)
            .and_then(|layout| layout.fields.get(&normalize_name(&field.name)))
            .copied()
            .ok_or_else(|| format!("record field `{}` is not known", field.name))
    }

    fn emit_deref_assignment(&mut self, target: &SemLValue, value: &SemExpr) -> Result<(), String> {
        let Some(deref) = self.classifier().pointer_deref_lvalue(target) else {
            return Err("expected pointer dereference target".to_string());
        };
        let pointee_width =
            self.materialize_pointer_deref_address(deref, NativeAddressDestination::ArrayAddr)?;
        match pointee_width {
            1 | 2 => {
                if pointee_width == 2
                    && self.emit_deref_expr_to_target(value, native_args_slot(2))?
                {
                    self.materialize_pointer_deref_address(
                        deref,
                        NativeAddressDestination::ArrayAddr,
                    )?;
                    self.materialize_args_to_array_addr_element(pointee_width)?;
                    return Ok(());
                }
                if !self.materialize_value_to_array_addr_element(value, pointee_width)? {
                    return Err(
                        "only byte and word pointer dereference values are supported".to_string(),
                    );
                }
            }
            _ => {
                return Err(
                    "only byte and word pointer dereference assignments are supported".to_string(),
                );
            }
        }
        Ok(())
    }

    fn emit_deref_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let Some(deref) = self.classifier().pointer_deref_expr(expr) else {
            return Ok(false);
        };
        let pointee_width =
            self.materialize_pointer_deref_address(deref, NativeAddressDestination::ArrayAddr)?;
        if pointee_width != target.width {
            return Err("pointer dereference assignment width mismatch".to_string());
        }
        match self.materialize_array_addr_element_to_target(target)? {
            true => {}
            _ => {
                return Err(
                    "only byte and word pointer dereference reads are supported".to_string()
                );
            }
        }
        Ok(true)
    }

    fn emit_deref_expr_to_a(&mut self, expr: &SemExpr) -> Result<bool, String> {
        let Some(deref) = self.classifier().pointer_deref_expr(expr) else {
            return Ok(false);
        };
        let pointee_width =
            self.materialize_pointer_deref_address(deref, NativeAddressDestination::ArrayAddr)?;
        if !matches!(pointee_width, 1 | 2) {
            return Err(
                "only byte and word pointer dereference expressions are supported".to_string(),
            );
        }
        self.materialize_array_addr_element_to_a();
        Ok(true)
    }

    fn emit_array_index_expr_to_a(&mut self, expr: &SemExpr) -> Result<bool, String> {
        let Some(indexed) = self.classifier().array_index_access(expr)? else {
            return Ok(false);
        };
        if !matches!(indexed.element_width, 1 | 2) {
            return Err("only byte and word array reads can materialize to A".to_string());
        }
        match indexed.storage {
            CodegenArrayStorage::Inline if indexed.element_width == 1 => {
                self.materialize_inline_array_byte_to_a(indexed)?
            }
            CodegenArrayStorage::Inline => {
                self.emit_array_index_to_x(indexed.index, indexed.element_width)?;
                self.emit_lda_addr_x(indexed.slot.address);
            }
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                self.materialize_pointer_backed_array_index_address(
                    indexed,
                    NativeAddressDestination::ArrayAddr,
                )?;
                self.materialize_array_addr_element_to_a();
            }
        }
        Ok(true)
    }

    fn emit_array_index_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let Some(indexed) = self.classifier().array_index_access(expr)? else {
            return Ok(false);
        };
        if indexed.element_width != target.width {
            return Err("array read width mismatch is not supported".to_string());
        }
        if indexed.element_width != 2 {
            return Ok(false);
        }
        match indexed.storage {
            CodegenArrayStorage::Inline => {
                self.materialize_inline_array_word_to_target(indexed, target.clone())?;
            }
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                self.materialize_pointer_backed_array_index_address(
                    indexed,
                    NativeAddressDestination::ArrayAddr,
                )?;
                self.materialize_array_addr_element_to_target(target)?;
            }
        }
        Ok(true)
    }

    fn emit_array_index_to_x(&mut self, index: &SemExpr, element_width: u16) -> Result<(), String> {
        match element_width {
            1 => self.emit_byte_expr_to_x(index),
            2 => {
                self.emit_array_index_to_a(index, element_width)?;
                self.emit_tax();
                Ok(())
            }
            _ => Err("only byte and word array indexes are supported".to_string()),
        }
    }

    fn emit_array_index_to_a(&mut self, index: &SemExpr, element_width: u16) -> Result<(), String> {
        if expr_width(index) == Some(2) {
            let target = native_afcur_slot(2);
            if self.materialize_value_to_target(index, target.clone())?
                || self.emit_expr_to_target(index, target.clone())?
            {
                self.emit_lda_addr(target.address);
            } else {
                return Err("only byte and word array indexes are supported".to_string());
            }
        } else {
            self.emit_byte_expr_to_a(index)?;
        }
        match element_width {
            1 => {}
            2 => self.emit_asl_a(),
            _ => return Err("only byte and word array indexes are supported".to_string()),
        }
        Ok(())
    }

    fn expr_contains_routine_call(&self, expr: &SemExpr) -> bool {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
                self.expr_contains_routine_call(expr)
            }
            SemExprKind::Binary { left, right, .. } => {
                self.expr_contains_routine_call(left) || self.expr_contains_routine_call(right)
            }
            SemExprKind::Call(call) => {
                let is_index_call = self.classifier().is_array_index_call(call)
                    || self.classifier().is_pointer_index_call(call);
                !is_index_call
                    || call
                        .args
                        .iter()
                        .any(|arg| self.expr_contains_routine_call(arg))
            }
            SemExprKind::LValue(lvalue)
            | SemExprKind::AddressOf(lvalue)
            | SemExprKind::ArrayDecay(SemArrayDecay { array: lvalue, .. }) => {
                self.lvalue_contains_routine_call(lvalue)
            }
            SemExprKind::ImplicitAddressOf(address) => {
                self.lvalue_contains_routine_call(&address.place)
            }
            SemExprKind::Missing
            | SemExprKind::Raw(_)
            | SemExprKind::UnresolvedName(_)
            | SemExprKind::CurrentLocation
            | SemExprKind::Literal(_)
            | SemExprKind::Symbol(_)
            | SemExprKind::AddressOfSymbol(_) => false,
        }
    }

    fn lvalue_contains_routine_call(&self, lvalue: &SemLValue) -> bool {
        match &lvalue.kind {
            SemLValueKind::Deref { pointer } => self.expr_contains_routine_call(pointer),
            SemLValueKind::Index { base, index, .. } => {
                self.expr_contains_routine_call(base) || self.expr_contains_routine_call(index)
            }
            SemLValueKind::Field { base, .. } => self.lvalue_contains_routine_call(base),
            SemLValueKind::Symbol(_) | SemLValueKind::UnresolvedName(_) => false,
        }
    }

    fn emit_compound_assignment(
        &mut self,
        target: &SemLValue,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<(), String> {
        if self.classifier().pointer_deref_lvalue(target).is_some() {
            return self.emit_deref_compound_assignment(target, op, value);
        }
        if self.emit_pointer_index_compound_assignment(target, op, value)? {
            return Ok(());
        }
        if self.emit_dynamic_array_compound_assignment(target, op, value)? {
            return Ok(());
        }
        let target = self.classifier().required_lvalue_slot(target)?;
        match target.width {
            1 => self.emit_byte_compound_assignment(target, op, value),
            2 => self.emit_word_compound_assignment(target, op, value),
            _ => Err("only byte and word compound assignments are supported".to_string()),
        }
    }

    fn emit_pointer_index_compound_assignment(
        &mut self,
        target: &SemLValue,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<bool, String> {
        let Some(indexed) = self.classifier().pointer_index_lvalue(target)? else {
            return Ok(false);
        };
        let pointee_width =
            self.materialize_pointer_index_address(indexed, NativeAddressDestination::ArrayAddr)?;
        match pointee_width {
            1 => self.emit_byte_deref_compound_assignment(op, value)?,
            _ => {
                return Err(
                    "only byte pointer indexed compound assignments are supported".to_string(),
                );
            }
        }
        Ok(true)
    }

    fn emit_dynamic_array_compound_assignment(
        &mut self,
        target: &SemLValue,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<bool, String> {
        let Some(indexed) = self.classifier().array_index_lvalue(target)? else {
            return Ok(false);
        };
        if indexed.element_width != 1 {
            return Err("only byte dynamic array compound assignments are supported".to_string());
        }
        if self.expr_contains_routine_call(indexed.index) {
            return Err(
                "dynamic array compound assignment indexes cannot call routines yet".to_string(),
            );
        }

        match indexed.storage {
            CodegenArrayStorage::Inline => {
                self.emit_inline_array_byte_compound_assignment(indexed, op, value)?;
            }
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                self.materialize_pointer_backed_array_index_address(
                    indexed,
                    NativeAddressDestination::ArrayAddr,
                )?;
                self.emit_byte_deref_compound_assignment(op, value)?;
            }
        }
        Ok(true)
    }

    fn emit_inline_array_byte_compound_assignment(
        &mut self,
        indexed: NativeArrayIndexAccess<'_>,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<(), String> {
        if indexed.storage != CodegenArrayStorage::Inline || indexed.element_width != 1 {
            return Err("expected byte inline array compound assignment".to_string());
        }
        if let Some(index) = literal_word(indexed.index) {
            let Some(array) = indexed.slot.array else {
                return Err("inline array compound assignment lost array metadata".to_string());
            };
            if array.len > 0 && index >= array.len {
                return Err(format!(
                    "array constant index {} is out of bounds {}",
                    index, array.len
                ));
            }
            let target = NativeResolvedSlot {
                address: indexed.slot.address + index,
                width: indexed.element_width,
                pointee_width: None,
                record: indexed.slot.record,
            };
            return self.emit_byte_compound_assignment(target, op, value);
        }

        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {}
            _ => {
                return Err(
                    "only +=, -=, and logic byte dynamic array compound assignments are supported"
                        .to_string(),
                );
            }
        }

        let staged_operand = !self.byte_compound_operand_is_direct(value, op)?;
        if staged_operand {
            self.emit_byte_expr_to_a(value)?;
            self.emit_sta_afcur();
        }
        self.emit_array_index_to_x(indexed.index, indexed.element_width)?;
        self.emit_lda_addr_x(indexed.slot.address);
        match op {
            BinaryOp::Add => {
                self.emit_clc();
                if staged_operand {
                    self.emit_adc_afcur();
                } else {
                    self.emit_adc_byte_expr(value)?;
                }
            }
            BinaryOp::Sub => {
                self.emit_sec();
                if staged_operand {
                    self.emit_sbc_afcur();
                } else {
                    self.emit_sbc_byte_expr(value)?;
                }
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                if staged_operand {
                    self.apply_logic_byte_source(
                        op,
                        NativeByteSource::Storage {
                            address: u16::from(runtime_zp::AFCUR.address()),
                        },
                    );
                } else {
                    self.emit_logic_byte_expr(op, value)?;
                }
            }
            _ => unreachable!("dynamic array compound operator already matched"),
        }
        self.emit_sta_addr_x(indexed.slot.address);
        Ok(())
    }

    fn emit_deref_compound_assignment(
        &mut self,
        target: &SemLValue,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<(), String> {
        let Some(deref) = self.classifier().pointer_deref_lvalue(target) else {
            return Err("expected pointer dereference target".to_string());
        };
        let pointee_width =
            self.materialize_pointer_deref_address(deref, NativeAddressDestination::ArrayAddr)?;
        match pointee_width {
            1 => self.emit_byte_deref_compound_assignment(op, value),
            _ => {
                Err("only byte pointer dereference compound assignments are supported".to_string())
            }
        }
    }

    fn emit_byte_deref_compound_assignment(
        &mut self,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<(), String> {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                if !self.byte_compound_operand_is_direct(value, op)? {
                    self.materialize_byte_expr_preserving_array_addr_to_afcur(value)?;
                    self.ensure_y_zero();
                    self.emit_lda_array_addr_indirect_y();
                    match op {
                        BinaryOp::Add => {
                            self.emit_clc();
                            self.emit_adc_afcur();
                        }
                        BinaryOp::Sub => {
                            self.emit_sec();
                            self.emit_sbc_afcur();
                        }
                        BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                            self.apply_logic_byte_source(
                                op,
                                NativeByteSource::Storage {
                                    address: u16::from(runtime_zp::AFCUR.address()),
                                },
                            );
                        }
                        _ => unreachable!("dynamic deref compound operator already matched"),
                    }
                    self.ensure_y_zero();
                    self.emit_sta_array_addr_indirect_y();
                    return Ok(());
                }
                self.ensure_y_zero();
                self.emit_lda_array_addr_indirect_y();
                if op == BinaryOp::Add {
                    self.emit_clc();
                    self.emit_adc_byte_expr(value)?;
                } else if op == BinaryOp::Sub {
                    self.emit_sec();
                    self.emit_sbc_byte_expr(value)?;
                } else {
                    self.emit_logic_byte_expr(op, value)?;
                }
                self.ensure_y_zero();
                self.emit_sta_array_addr_indirect_y();
                Ok(())
            }
            _ => Err(
                "only +=, -=, and logic byte pointer dereference assignments are supported"
                    .to_string(),
            ),
        }
    }

    fn byte_compound_operand_is_direct(
        &self,
        value: &SemExpr,
        op: BinaryOp,
    ) -> Result<bool, String> {
        if self.classifier().byte_operand_source(value)?.is_some() {
            return Ok(true);
        }
        Ok(op == BinaryOp::Add && byte_lsh_result_is_zero(value))
    }

    fn materialize_byte_expr_preserving_array_addr_to_afcur(
        &mut self,
        value: &SemExpr,
    ) -> Result<(), String> {
        self.materialize_array_addr_to_stack();
        let result = self.emit_byte_expr_to_a(value);
        match result {
            Ok(()) => {
                self.emit_sta_afcur();
                self.materialize_stack_to_array_addr();
                Ok(())
            }
            Err(reason) => {
                self.materialize_stack_to_array_addr();
                Err(reason)
            }
        }
    }

    fn emit_byte_compound_assignment(
        &mut self,
        target: NativeResolvedSlot,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<(), String> {
        match op {
            BinaryOp::Add => {
                if literal_byte(value) == Some(1) {
                    self.emit_inc_addr(target.address);
                    return Ok(());
                }
                self.emit_clc();
                self.emit_lda_addr(target.address);
                self.emit_adc_byte_expr(value)?;
                self.emit_sta_addr(target.address);
                Ok(())
            }
            BinaryOp::Sub => {
                if literal_byte(value) == Some(1) {
                    self.emit_dec_addr(target.address);
                    return Ok(());
                }
                self.emit_sec();
                self.emit_lda_addr(target.address);
                self.emit_sbc_byte_expr(value)?;
                self.emit_sta_addr(target.address);
                Ok(())
            }
            BinaryOp::Rsh => {
                let count = literal_byte(value).ok_or_else(|| {
                    "only constant byte RSH compound counts are supported".to_string()
                })?;
                for _ in 0..count.min(8) {
                    self.emit_lsr_addr(target.address);
                }
                if count >= 8 {
                    self.emit_lda_imm(0);
                    self.emit_sta_addr(target.address);
                }
                Ok(())
            }
            BinaryOp::Div | BinaryOp::Mod => {
                self.emit_byte_runtime_binary_slot_to_a(op, target.clone(), value)?;
                self.emit_sta_addr(target.address);
                Ok(())
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                self.emit_lda_addr(target.address);
                self.emit_logic_byte_expr(op, value)?;
                self.emit_sta_addr(target.address);
                Ok(())
            }
            _ => Err(
                "only +=, -=, RSH, /, MOD, and byte logic compound assignments are supported"
                    .to_string(),
            ),
        }
    }

    fn emit_word_compound_assignment(
        &mut self,
        target: NativeResolvedSlot,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<(), String> {
        match op {
            BinaryOp::Add | BinaryOp::Sub => {
                if self.emit_word_compound_byte_expr(target.clone(), op, value)? {
                    return Ok(());
                }
                let staged_operand = self.stage_word_compound_operand(value)?;
                let is_add = op == BinaryOp::Add;
                if is_add {
                    self.emit_clc();
                } else {
                    self.emit_sec();
                }
                self.emit_lda_addr(target.address);
                if let Some(operand) = &staged_operand {
                    self.emit_word_compound_slot_operand(operand, 0, is_add);
                } else {
                    self.emit_word_compound_operand(value, 0, is_add)?;
                }
                self.emit_sta_addr(target.address);
                self.emit_lda_addr(target.address + 1);
                if let Some(operand) = &staged_operand {
                    self.emit_word_compound_slot_operand(operand, 1, is_add);
                } else {
                    self.emit_word_compound_operand(value, 1, is_add)?;
                }
                self.emit_sta_addr(target.address + 1);
                Ok(())
            }
            BinaryOp::Rsh => {
                let count = literal_byte(value).ok_or_else(|| {
                    "only constant word RSH compound counts are supported".to_string()
                })?;
                if count >= 16 {
                    self.emit_lda_imm(0);
                    self.emit_sta_addr(target.address + 1);
                    self.emit_sta_addr(target.address);
                    return Ok(());
                }
                for _ in 0..count {
                    self.emit_lsr_addr(target.address + 1);
                    self.emit_ror_addr(target.address);
                }
                Ok(())
            }
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                self.emit_word_runtime_binary_slot_to_target(op, target, value)
            }
            _ => Err(
                "only +=, -=, RSH, *, /, and MOD word compound assignments are supported"
                    .to_string(),
            ),
        }
    }

    fn emit_word_compound_byte_expr(
        &mut self,
        target: NativeResolvedSlot,
        op: BinaryOp,
        value: &SemExpr,
    ) -> Result<bool, String> {
        if expr_width(value) != Some(1) {
            return Ok(false);
        }
        if self.classifier().byte_operand_source(value)?.is_some() {
            return Ok(false);
        }
        if !self.emit_expr_to_a(value)? {
            return Ok(false);
        }
        self.emit_sta_element_addr();

        let is_add = op == BinaryOp::Add;
        if is_add {
            self.emit_clc();
        } else {
            self.emit_sec();
        }
        self.emit_lda_addr(target.address);
        if is_add {
            self.emit_adc_element_addr();
        } else {
            self.emit_sbc_element_addr();
        }
        self.emit_sta_addr(target.address);
        self.emit_lda_addr(target.address + 1);
        if is_add {
            self.emit_adc_imm(0);
        } else {
            self.emit_sbc_imm(0);
        }
        self.emit_sta_addr(target.address + 1);
        Ok(true)
    }

    fn stage_word_compound_operand(
        &mut self,
        value: &SemExpr,
    ) -> Result<Option<NativeResolvedSlot>, String> {
        if expr_width(value) != Some(2) || self.word_compound_operand_is_direct(value)? {
            return Ok(None);
        }
        if self.expr_contains_routine_call(value) {
            return Err(format!(
                "word compound operands with calls are not supported ({})",
                native_expr_debug_name(value)
            ));
        }

        let target = native_afcur_slot(2);
        if self.emit_record_field_expr_to_target(value, target.clone())?
            || self.emit_pointer_index_expr_to_target(value, target.clone())?
            || self.emit_array_index_expr_to_target(value, target.clone())?
            || self.emit_deref_expr_to_target(value, target.clone())?
        {
            return Ok(Some(target));
        }
        Ok(None)
    }

    fn emit_word_compound_slot_operand(
        &mut self,
        slot: &NativeResolvedSlot,
        byte_index: u16,
        is_add: bool,
    ) {
        let address = slot.address + byte_index;
        if is_add {
            self.emit_adc_addr(address);
        } else {
            self.emit_sbc_addr(address);
        }
    }

    fn emit_word_compound_operand(
        &mut self,
        value: &SemExpr,
        byte_index: u16,
        is_add: bool,
    ) -> Result<(), String> {
        if let Some(value) = literal_word(value) {
            let byte = if byte_index == 0 {
                (value & 0x00FF) as u8
            } else {
                (value >> 8) as u8
            };
            if is_add {
                self.emit_adc_imm(byte);
            } else {
                self.emit_sbc_imm(byte);
            }
            return Ok(());
        }
        if let Some(source) = self.classifier().compare_byte_source(value, byte_index)? {
            if is_add {
                self.apply_adc_byte_source(source);
            } else {
                self.apply_sbc_byte_source(source);
            }
            return Ok(());
        }

        let slot = self.classifier().required_addressable_slot(value)?;
        if byte_index >= slot.width {
            if is_add {
                self.emit_adc_imm(0);
            } else {
                self.emit_sbc_imm(0);
            }
            return Ok(());
        }
        if is_add {
            self.emit_adc_addr(slot.address + byte_index);
        } else {
            self.emit_sbc_addr(slot.address + byte_index);
        }
        Ok(())
    }

    fn emit_return_value(&mut self, value: &SemExpr) -> Result<(), String> {
        if let (Some(width), Some(value)) =
            (self.current_return_width, self.constant_return_word(value))
        {
            match width {
                1 => {
                    self.emit_lda_imm((value & 0x00FF) as u8);
                    self.emit_sta_args(0);
                    self.emit_rts();
                    return Ok(());
                }
                2 => {
                    self.emit_word_literal_to_target(value, native_args_slot(2));
                    self.emit_rts();
                    return Ok(());
                }
                _ => {}
            }
        }

        if expr_width(value) == Some(1)
            && self.materialize_value_to_target(value, native_args_slot(1))?
        {
            if self.current_return_width == Some(2) {
                self.emit_lda_imm(0);
                self.emit_sta_args(1);
            }
            self.emit_rts();
            return Ok(());
        }

        if let Some(width) = expr_width(value)
            && width == 1
            && self.emit_expr_to_a(value)?
        {
            self.emit_sta_args(0);
            if self.current_return_width == Some(2) {
                self.emit_lda_imm(0);
                self.emit_sta_args(1);
            }
            self.emit_rts();
            return Ok(());
        }
        if expr_width(value) == Some(2)
            && self.materialize_value_to_target(value, native_args_slot(2))?
        {
            self.emit_rts();
            return Ok(());
        }
        if expr_width(value) == Some(2) && self.emit_expr_to_target(value, native_args_slot(2))? {
            self.emit_rts();
            return Ok(());
        }

        let source = self.classifier().required_addressable_slot(value)?;
        let source_width = source.width;
        if !self.materialize_slot_to_target(source, native_args_slot(source_width))? {
            return Err("return values wider than a word are not supported".to_string());
        }
        self.emit_rts();
        Ok(())
    }

    fn emit_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if let Some(call) = self.classifier().routine_call_expr(expr) {
            self.emit_call(call)?;
            self.materialize_return_slot_to_target(target)?;
            return Ok(true);
        }
        if target.width == 2 {
            if self.emit_word_unary_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_word_runtime_binary_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_word_shift_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_word_logic_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_word_binary_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_deref_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_record_field_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_pointer_index_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if self.emit_array_index_expr_to_target(expr, target.clone())? {
                return Ok(true);
            }
            if let Some(source) = self.classifier().addressable_slot(expr)? {
                if source.width != 2 {
                    return Err("word expression source width mismatch".to_string());
                }
                self.materialize_slot_to_target(source, target)?;
                return Ok(true);
            }
            if !self.emit_expr_to_a(expr)? {
                return Ok(false);
            }
            self.emit_sta_addr(target.address);
            self.emit_lda_imm(0);
            self.emit_sta_addr(target.address + 1);
            return Ok(true);
        }
        if target.width != 1 {
            return Ok(false);
        }
        if self.emit_word_runtime_binary_expr_to_a(expr)? {
            self.emit_sta_addr(target.address);
            return Ok(true);
        }
        if self.emit_record_field_expr_to_target(expr, target.clone())? {
            return Ok(true);
        }
        if !self.emit_expr_to_a(expr)? {
            return Ok(false);
        }
        self.emit_sta_addr(target.address);
        Ok(true)
    }

    fn emit_word_binary_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let SemExprKind::Binary {
            op: op @ (BinaryOp::Add | BinaryOp::Sub),
            left,
            right,
        } = &expr.kind
        else {
            return Ok(false);
        };

        if let Some(left) = self.classifier().addressable_slot(left)? {
            if left.width != 2 {
                return Ok(false);
            }
            if self.word_compound_operand_is_direct(right)? {
                let is_add = *op == BinaryOp::Add;
                if is_add {
                    self.emit_clc();
                } else {
                    self.emit_sec();
                }
                self.emit_lda_addr(left.address);
                self.emit_word_compound_operand(right, 0, is_add)?;
                self.emit_sta_addr(target.address);
                self.emit_lda_addr(left.address + 1);
                self.emit_word_compound_operand(right, 1, is_add)?;
                self.emit_sta_addr(target.address + 1);
                return Ok(true);
            }
            self.materialize_slot_to_target(left, target.clone())?;
            self.emit_word_compound_assignment(target, *op, right)?;
            return Ok(true);
        }

        if expr_width(expr) == Some(2) && expr_width(left) == Some(1) {
            if !self.emit_byte_expr_to_word_target(left, target.clone())? {
                return Ok(false);
            }
            self.emit_word_compound_assignment(target, *op, right)?;
            return Ok(true);
        }

        if expr_width(left) != Some(2) || !self.emit_expr_to_target(left, target.clone())? {
            return Ok(false);
        }
        self.emit_word_compound_assignment(target, *op, right)?;
        Ok(true)
    }

    fn emit_byte_expr_to_word_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if target.width != 2 || expr_width(expr) != Some(1) {
            return Ok(false);
        }
        if !self.materialize_byte_value_to_a(expr)? && !self.emit_expr_to_a(expr)? {
            return Ok(false);
        }
        self.emit_sta_addr(target.address);
        self.emit_lda_imm(0);
        self.emit_sta_addr(target.address + 1);
        Ok(true)
    }

    fn emit_word_logic_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let SemExprKind::Binary {
            op: op @ (BinaryOp::And | BinaryOp::Or | BinaryOp::Xor),
            left,
            right,
        } = &expr.kind
        else {
            return Ok(false);
        };

        if expr_width(expr) != Some(2) || target.width != 2 {
            return Ok(false);
        }

        let left = self.word_logic_operand_sources(left, native_args_offset_slot(4, 2)?)?;
        let right = self.word_logic_operand_sources(right, native_args_offset_slot(6, 2)?)?;

        self.materialize_byte_source_to_register(left.low, NativeByteRegister::A)?;
        self.apply_logic_byte_source(*op, right.low);
        self.emit_sta_addr(target.address);

        self.materialize_byte_source_to_register(left.high, NativeByteRegister::A)?;
        self.apply_logic_byte_source(*op, right.high);
        self.emit_sta_addr(target.address + 1);

        Ok(true)
    }

    fn emit_word_unary_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let SemExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } = &expr.kind
        else {
            return Ok(false);
        };

        if expr_width(expr) != Some(2) || target.width != 2 {
            return Ok(false);
        }

        let source = self
            .word_value_sources(expr, native_args_offset_slot(4, 2)?)?
            .ok_or_else(|| {
                format!(
                    "only word unary negation operands are supported ({})",
                    native_expr_debug_name(expr)
                )
            })?;

        self.emit_sec();
        self.emit_lda_imm(0);
        self.apply_sbc_byte_source(source.low);
        self.emit_sta_addr(target.address);
        self.emit_lda_imm(0);
        self.apply_sbc_byte_source(source.high);
        self.emit_sta_addr(target.address + 1);
        Ok(true)
    }

    fn emit_word_runtime_binary_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let SemExprKind::Binary {
            op: op @ (BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod),
            left,
            right,
        } = &expr.kind
        else {
            return Ok(false);
        };

        if expr_width(expr) != Some(2) || target.width != 2 {
            return Ok(false);
        }

        let right = self
            .word_value_sources(right, native_args_offset_slot(6, 2)?)?
            .ok_or_else(|| {
                format!(
                    "only word runtime right operands are supported ({})",
                    native_expr_debug_name(right)
                )
            })?;
        self.materialize_byte_source_to_register(right.low, NativeByteRegister::A)?;
        self.emit_sta_afcur();
        self.materialize_byte_source_to_register(right.high, NativeByteRegister::A)?;
        self.emit_sta_afcur_high();

        let left_source = self
            .word_value_sources(left, native_args_offset_slot(4, 2)?)?
            .ok_or_else(|| {
                format!(
                    "only word runtime left operands are supported ({})",
                    native_expr_debug_name(left)
                )
            })?;
        self.materialize_byte_source_to_register(left_source.high, NativeByteRegister::X)?;
        self.materialize_byte_source_to_register(left_source.low, NativeByteRegister::A)?;

        let helper = match op {
            BinaryOp::Mul => RuntimeHelperSlot::Mul,
            BinaryOp::Div => RuntimeHelperSlot::Div,
            BinaryOp::Mod => RuntimeHelperSlot::Mod,
            _ => unreachable!("word runtime operator checked by caller"),
        };
        self.emit_jsr_runtime_helper(self.runtime_helpers.target(helper), left.span);
        self.emit_sta_addr(target.address);
        self.emit_stx_addr(target.address + 1);
        Ok(true)
    }

    fn emit_word_runtime_binary_expr_to_a(&mut self, expr: &SemExpr) -> Result<bool, String> {
        let SemExprKind::Binary {
            op: op @ (BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod),
            left,
            right,
        } = &expr.kind
        else {
            return Ok(false);
        };

        if expr_width(expr) != Some(2) {
            return Ok(false);
        }

        let right = self
            .word_value_sources(right, native_args_offset_slot(6, 2)?)?
            .ok_or_else(|| {
                format!(
                    "only word runtime right operands are supported ({})",
                    native_expr_debug_name(right)
                )
            })?;
        self.materialize_byte_source_to_register(right.low, NativeByteRegister::A)?;
        self.emit_sta_afcur();
        self.materialize_byte_source_to_register(right.high, NativeByteRegister::A)?;
        self.emit_sta_afcur_high();

        let left_source = self
            .word_value_sources(left, native_args_offset_slot(4, 2)?)?
            .ok_or_else(|| {
                format!(
                    "only word runtime left operands are supported ({})",
                    native_expr_debug_name(left)
                )
            })?;
        self.materialize_byte_source_to_register(left_source.high, NativeByteRegister::X)?;
        self.materialize_byte_source_to_register(left_source.low, NativeByteRegister::A)?;

        let helper = match op {
            BinaryOp::Mul => RuntimeHelperSlot::Mul,
            BinaryOp::Div => RuntimeHelperSlot::Div,
            BinaryOp::Mod => RuntimeHelperSlot::Mod,
            _ => unreachable!("word runtime operator checked by caller"),
        };
        self.emit_jsr_runtime_helper(self.runtime_helpers.target(helper), left.span);
        Ok(true)
    }

    fn emit_word_runtime_binary_slot_to_target(
        &mut self,
        op: BinaryOp,
        target: NativeResolvedSlot,
        right: &SemExpr,
    ) -> Result<(), String> {
        if target.width != 2 {
            return Err("only word runtime compound assignments are supported".to_string());
        }

        let right_span = right.span;
        let right = self
            .word_value_sources(right, native_args_offset_slot(6, 2)?)?
            .ok_or_else(|| {
                format!(
                    "only word runtime compound operands are supported ({})",
                    native_expr_debug_name(right)
                )
            })?;
        self.materialize_byte_source_to_register(right.low, NativeByteRegister::A)?;
        self.emit_sta_afcur();
        self.materialize_byte_source_to_register(right.high, NativeByteRegister::A)?;
        self.emit_sta_afcur_high();

        self.emit_ldx_addr(target.address + 1);
        self.emit_lda_addr(target.address);

        let helper = match op {
            BinaryOp::Mul => RuntimeHelperSlot::Mul,
            BinaryOp::Div => RuntimeHelperSlot::Div,
            BinaryOp::Mod => RuntimeHelperSlot::Mod,
            _ => unreachable!("word runtime compound operator checked by caller"),
        };
        self.emit_jsr_runtime_helper(self.runtime_helpers.target(helper), right_span);
        self.emit_sta_addr(target.address);
        self.emit_stx_addr(target.address + 1);
        Ok(())
    }

    fn emit_word_shift_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let SemExprKind::Binary {
            op: op @ (BinaryOp::Lsh | BinaryOp::Rsh),
            left,
            right,
        } = &expr.kind
        else {
            return Ok(false);
        };

        if expr_width(expr) != Some(2) || target.width != 2 {
            return Ok(false);
        }

        if !self.materialize_byte_value_to_a(right)? && !self.emit_expr_to_a(right)? {
            return Err("only byte word-shift counts are supported".to_string());
        }
        self.emit_sta_afcur();

        if let Some(source) = self.word_value_sources(left, native_args_offset_slot(4, 2)?)? {
            self.materialize_byte_source_to_register(source.high, NativeByteRegister::X)?;
            self.materialize_byte_source_to_register(source.low, NativeByteRegister::A)?;
        } else {
            return Err(format!(
                "only word shift left operands are supported ({})",
                native_expr_debug_name(left)
            ));
        }

        let helper = match op {
            BinaryOp::Lsh => RuntimeHelperSlot::Lsh,
            BinaryOp::Rsh => RuntimeHelperSlot::Rsh,
            _ => unreachable!("word shift operator checked by caller"),
        };
        self.emit_jsr_runtime_helper(self.runtime_helpers.target(helper), left.span);
        self.emit_sta_addr(target.address);
        self.emit_stx_addr(target.address + 1);
        Ok(true)
    }

    fn word_value_sources(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<Option<NativeWordSource>, String> {
        if let Some(low) = self.classifier().compare_byte_source(expr, 0)?
            && let Some(high) = self.classifier().compare_byte_source(expr, 1)?
        {
            return Ok(Some(NativeWordSource { low, high }));
        }
        if expr_width(expr) == Some(1) {
            if !self.materialize_byte_value_to_a(expr)? && !self.emit_expr_to_a(expr)? {
                return Ok(None);
            }
            self.emit_sta_addr(target.address);
            self.emit_lda_imm(0);
            self.emit_sta_addr(target.address + 1);
            return Ok(Some(NativeWordSource {
                low: NativeByteSource::Storage {
                    address: target.address,
                },
                high: NativeByteSource::Storage {
                    address: target.address + 1,
                },
            }));
        }
        if expr_width(expr) == Some(2)
            && (self.materialize_value_to_target(expr, target.clone())?
                || self.emit_expr_to_target(expr, target.clone())?)
        {
            return Ok(Some(NativeWordSource {
                low: NativeByteSource::Storage {
                    address: target.address,
                },
                high: NativeByteSource::Storage {
                    address: target.address + 1,
                },
            }));
        }
        Ok(None)
    }

    fn word_logic_operand_sources(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<NativeWordSource, String> {
        if let Some(low) = self.classifier().compare_byte_source(expr, 0)?
            && let Some(high) = self.classifier().compare_byte_source(expr, 1)?
        {
            return Ok(NativeWordSource { low, high });
        }
        if expr_width(expr) == Some(1) {
            if !self.materialize_byte_value_to_a(expr)? && !self.emit_expr_to_a(expr)? {
                return Err(format!(
                    "only byte-to-word logic operands are supported ({})",
                    native_expr_debug_name(expr)
                ));
            }
            self.emit_sta_addr(target.address);
            self.emit_lda_imm(0);
            self.emit_sta_addr(target.address + 1);
            return Ok(NativeWordSource {
                low: NativeByteSource::Storage {
                    address: target.address,
                },
                high: NativeByteSource::Storage {
                    address: target.address + 1,
                },
            });
        }
        if expr_width(expr) == Some(2)
            && (self.materialize_value_to_target(expr, target.clone())?
                || self.emit_expr_to_target(expr, target.clone())?)
        {
            return Ok(NativeWordSource {
                low: NativeByteSource::Storage {
                    address: target.address,
                },
                high: NativeByteSource::Storage {
                    address: target.address + 1,
                },
            });
        }
        Err(format!(
            "only word logic operands are supported ({})",
            native_expr_debug_name(expr)
        ))
    }

    fn word_compound_operand_is_direct(&self, value: &SemExpr) -> Result<bool, String> {
        if literal_word(value).is_some() {
            return Ok(true);
        }
        if self
            .classifier()
            .compare_byte_source(value, 0)?
            .zip(self.classifier().compare_byte_source(value, 1)?)
            .is_some()
        {
            return Ok(true);
        }
        self.classifier()
            .addressable_slot(value)
            .map(|slot| slot.is_some())
    }

    fn emit_expr_to_a(&mut self, expr: &SemExpr) -> Result<bool, String> {
        if let Some(value) = literal_byte(expr) {
            self.emit_lda_imm(value);
            return Ok(true);
        }
        if self.materialize_byte_value_to_a(expr)? {
            return Ok(true);
        }
        if self.materialize_byte_product_to_ax(expr)? {
            return Ok(true);
        }
        if self.emit_call_result_to_a(expr)? {
            return Ok(true);
        }
        if self.emit_deref_expr_to_a(expr)? {
            return Ok(true);
        }
        if self.emit_pointer_index_expr_to_a(expr)? {
            return Ok(true);
        }
        if self.emit_array_index_expr_to_a(expr)? {
            return Ok(true);
        }
        if self.emit_record_field_expr_to_a(expr)? {
            return Ok(true);
        }
        let SemExprKind::Binary { op, left, right } = &expr.kind else {
            return Ok(false);
        };
        match op {
            BinaryOp::Add => {
                if let Some(left) = self.classifier().addressable_slot(left)? {
                    if left.width != 1 {
                        return Err("only byte addition expressions are supported".to_string());
                    }
                    if byte_lsh_result_is_zero(right) {
                        self.emit_lda_addr(left.address);
                        return Ok(true);
                    }
                    self.emit_clc();
                    self.emit_lda_addr(left.address);
                } else if self.emit_expr_to_a(left)? {
                    if byte_lsh_result_is_zero(right) {
                        return Ok(true);
                    }
                    self.emit_sta_element_addr();
                    self.emit_clc();
                    self.emit_lda_element_addr();
                } else {
                    return Err("only byte addition expressions are supported".to_string());
                }
                self.emit_adc_byte_expr(right)?;
                Ok(true)
            }
            BinaryOp::Sub => {
                if let Some(left) = self.classifier().addressable_slot(left)? {
                    if left.width != 1 {
                        return Err("only byte subtraction expressions are supported".to_string());
                    }
                    self.emit_sec();
                    self.emit_lda_addr(left.address);
                } else if self.emit_expr_to_a(left)? {
                    self.emit_sta_element_addr();
                    self.emit_sec();
                    self.emit_lda_element_addr();
                } else {
                    return Err("only byte subtraction expressions are supported".to_string());
                }
                self.emit_sbc_byte_expr(right)?;
                Ok(true)
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                if let Some(left) = self.classifier().addressable_slot(left)? {
                    if left.width != 1 {
                        return Err("only byte logic expressions are supported".to_string());
                    }
                    self.emit_lda_addr(left.address);
                } else if self.emit_expr_to_a(left)? {
                    self.emit_sta_element_addr();
                    self.emit_lda_element_addr();
                } else {
                    return Err("only byte logic expressions are supported".to_string());
                }
                self.emit_logic_byte_expr(*op, right)?;
                Ok(true)
            }
            BinaryOp::Lsh => {
                if expr_width(left) != Some(1) {
                    return Err("only byte LSH expressions are supported".to_string());
                }
                if let Some(count) = literal_byte(right) {
                    if count >= 8 {
                        self.emit_lda_imm(0);
                        return Ok(true);
                    }
                    self.emit_lda_imm(count);
                } else if !self.materialize_byte_value_to_a(right)?
                    && !self.emit_expr_to_a(right)?
                {
                    return Err("only byte LSH counts are supported".to_string());
                }
                self.emit_sta_afcur();
                if !self.materialize_byte_value_to_a(left)? && !self.emit_expr_to_a(left)? {
                    return Err("only byte LSH expressions are supported".to_string());
                }
                self.emit_ldx_imm(0);
                self.emit_jsr_runtime_helper(
                    self.runtime_helpers.target(RuntimeHelperSlot::Lsh),
                    right.span,
                );
                Ok(true)
            }
            BinaryOp::Rsh => {
                if let Some(count) = literal_byte(right) {
                    if count >= 8 {
                        self.emit_lda_imm(0);
                        return Ok(true);
                    }
                    if let Some(left) = self.classifier().addressable_slot(left)? {
                        if left.width != 1 {
                            return Err("only byte RSH expressions are supported".to_string());
                        }
                        self.emit_lda_addr(left.address);
                    } else if !self.emit_expr_to_a(left)? {
                        return Err("only byte RSH expressions are supported".to_string());
                    }
                    for _ in 0..count {
                        self.emit_lsr_a();
                    }
                    return Ok(true);
                }
                if !self.materialize_byte_value_to_a(right)? && !self.emit_expr_to_a(right)? {
                    return Err("only byte RSH counts are supported".to_string());
                }
                self.emit_sta_afcur();
                if !self.materialize_byte_value_to_a(left)? && !self.emit_expr_to_a(left)? {
                    return Err("only byte RSH expressions are supported".to_string());
                }
                self.emit_ldx_imm(0);
                self.emit_jsr_runtime_helper(
                    self.runtime_helpers.target(RuntimeHelperSlot::Rsh),
                    right.span,
                );
                Ok(true)
            }
            BinaryOp::Div | BinaryOp::Mod => {
                self.emit_byte_runtime_binary_expr_to_a(*op, left, right)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn emit_byte_runtime_binary_expr_to_a(
        &mut self,
        op: BinaryOp,
        left: &SemExpr,
        right: &SemExpr,
    ) -> Result<(), String> {
        if !native_byte_runtime_operand_is_supported(left)
            || !native_byte_runtime_operand_is_supported(right)
        {
            return Err("only byte runtime binary expressions are supported".to_string());
        }
        self.stage_byte_runtime_rhs(right)?;
        self.emit_byte_expr_to_a(left)?;
        self.emit_ldx_imm(0);
        self.emit_jsr_runtime_helper(
            self.runtime_helpers.target(byte_runtime_helper(op)?),
            left.span,
        );
        Ok(())
    }

    fn emit_byte_runtime_binary_slot_to_a(
        &mut self,
        op: BinaryOp,
        left: NativeResolvedSlot,
        right: &SemExpr,
    ) -> Result<(), String> {
        if left.width != 1 || !native_byte_runtime_operand_is_supported(right) {
            return Err("only byte runtime compound assignments are supported".to_string());
        }
        self.stage_byte_runtime_rhs(right)?;
        self.emit_lda_addr(left.address);
        self.emit_ldx_imm(0);
        self.emit_jsr_runtime_helper(
            self.runtime_helpers.target(byte_runtime_helper(op)?),
            right.span,
        );
        Ok(())
    }

    fn stage_byte_runtime_rhs(&mut self, right: &SemExpr) -> Result<(), String> {
        self.emit_byte_expr_to_a(right)?;
        self.emit_sta_afcur();
        self.emit_lda_imm(0);
        self.emit_sta_afcur_high();
        Ok(())
    }

    fn emit_call_result_to_a(&mut self, expr: &SemExpr) -> Result<bool, String> {
        let Some(call) = self.classifier().routine_call_expr(expr) else {
            return Ok(false);
        };
        self.emit_call(call)?;
        self.emit_lda_args(0);
        Ok(true)
    }

    fn emit_adc_byte_expr(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let Some(source) = self.classifier().byte_operand_source(expr)? {
            self.apply_adc_byte_source(source);
            return Ok(());
        }
        if byte_lsh_result_is_zero(expr) {
            self.emit_adc_imm(0);
            return Ok(());
        }
        self.emit_sta_element_addr();
        self.emit_byte_expr_to_a(expr)?;
        self.emit_sta_afcur();
        self.emit_lda_element_addr();
        self.emit_clc();
        self.emit_adc_afcur();
        Ok(())
    }

    fn emit_sbc_byte_expr(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let Some(source) = self.classifier().byte_operand_source(expr)? {
            self.apply_sbc_byte_source(source);
            return Ok(());
        }
        self.emit_sta_element_addr();
        self.emit_byte_expr_to_a(expr)?;
        self.emit_sta_afcur();
        self.emit_lda_element_addr();
        self.emit_sec();
        self.emit_sbc_afcur();
        Ok(())
    }

    fn emit_logic_byte_expr(&mut self, op: BinaryOp, expr: &SemExpr) -> Result<(), String> {
        if let Some(source) = self.classifier().byte_operand_source(expr)? {
            self.apply_logic_byte_source(op, source);
            return Ok(());
        }
        self.emit_sta_element_addr();
        self.emit_byte_expr_to_a(expr)?;
        self.emit_sta_afcur();
        self.emit_lda_element_addr();
        self.apply_logic_byte_source(
            op,
            NativeByteSource::Storage {
                address: u16::from(runtime_zp::AFCUR.address()),
            },
        );
        Ok(())
    }

    fn emit_call(&mut self, call: &SemCall) -> Result<(), String> {
        let (name, callee_id, target) = match &call.callee {
            SemCallable::User(symbol) => {
                let entry = self
                    .routine_entries
                    .get(&symbol.id)
                    .copied()
                    .ok_or_else(|| format!("routine `{}` has no native entry yet", symbol.name))?;
                (
                    symbol.name.as_str(),
                    Some(symbol.id),
                    NativeCallTarget::Address(entry),
                )
            }
            SemCallable::Builtin(symbol) => {
                let address = native_builtin_system_address(&symbol.name).ok_or_else(|| {
                    format!("builtin `{}` has no native system address", symbol.name)
                })?;
                (
                    symbol.name.as_str(),
                    Some(symbol.id),
                    NativeCallTarget::Address(address),
                )
            }
            SemCallable::Runtime {
                name,
                address: Some(address),
                ..
            } => (name.as_str(), None, NativeCallTarget::Address(*address)),
            SemCallable::Runtime { name, .. } => {
                return Err(format!("runtime call `{name}` has no native address"));
            }
            SemCallable::Indirect { target, .. } => {
                if self.emit_define_machine_call(target, call.span)? {
                    return Ok(());
                }
                return Err(
                    "only direct user, builtin, and runtime calls are supported".to_string()
                );
            }
        };
        let result = (|| {
            let arg_bytes = self.flatten_call_arg_bytes(callee_id, &call.args)?;
            if arg_bytes.len() > 2 {
                self.materialize_sargs_call_args(&arg_bytes)?;
            } else if call.args.len() > 2 {
                return Err(format!("routine `{name}` calls have unsupported args"));
            } else if arg_bytes.len() == 2 && call.args.len() == 1 {
                self.materialize_word_call_arg_to_ax(&call.args[0])?;
            } else if call.args.len() == 2 && self.emit_pointer_index_expr_to_a(&call.args[0])? {
                self.emit_sta_args(0);
                self.emit_byte_expr_to_x(&call.args[1])?;
                self.emit_lda_args(0);
            } else {
                if let Some(arg) = call.args.get(1) {
                    self.emit_byte_expr_to_x(arg)?;
                }
                if let Some(arg) = call.args.first() {
                    self.emit_byte_expr_to_a(arg)?;
                }
            }
            match target {
                NativeCallTarget::Address(address) => self.emit_jsr_addr(address),
            }
            Ok(())
        })();
        result.map_err(|err| format!("call `{name}`: {err}"))
    }

    fn emit_define_machine_call(&mut self, target: &SemExpr, span: Span) -> Result<bool, String> {
        let SemExprKind::Symbol(symbol) = &target.kind else {
            return Ok(false);
        };
        if symbol.class != SymbolClass::Define {
            return Ok(false);
        }
        let Some(items) = self.machine_define_items(symbol)? else {
            return Err(format!(
                "define `{}` has no native machine body",
                symbol.name
            ));
        };
        self.emit_machine_block(&items, span)?;
        Ok(true)
    }

    fn flatten_call_arg_bytes<'b>(
        &mut self,
        callee: Option<SymbolId>,
        args: &'b [SemExpr],
    ) -> Result<Vec<NativeCallArgByte<'b>>, String> {
        let mut bytes = Vec::new();
        let mut offset = 0u16;
        for (index, arg) in args.iter().enumerate() {
            let width = self.call_arg_width(callee, index, arg)?;
            for byte_index in 0..width {
                bytes.push(NativeCallArgByte {
                    expr: arg,
                    width,
                    byte_index,
                    offset,
                });
                offset = offset
                    .checked_add(1)
                    .ok_or_else(|| "call argument byte count overflow".to_string())?;
            }
        }
        Ok(bytes)
    }

    fn call_arg_width(
        &self,
        callee: Option<SymbolId>,
        index: usize,
        arg: &SemExpr,
    ) -> Result<u16, String> {
        if let Some(callee) = callee
            && let Some(width) = self.callee_param_width(callee, index)?
        {
            return Ok(width);
        }
        self.classifier()
            .value_width(arg)
            .map_err(|reason| format!("call argument width is not known: {reason}"))
    }

    fn callee_param_width(&self, callee: SymbolId, index: usize) -> Result<Option<u16>, String> {
        let Some(routine) = self
            .model
            .routines
            .iter()
            .find(|routine| routine.routine.symbol.id == callee)
            .map(|routine| routine.routine)
        else {
            return Ok(None);
        };
        let Some(param) = routine.params.get(index) else {
            return Ok(None);
        };
        match param.storage {
            SemParamStorage::Value => self
                .native_sem_type_width(&param.ty)
                .map(Some)
                .ok_or_else(|| format!("unsupported parameter type `{}`", param.symbol.name)),
            SemParamStorage::Array => Ok(Some(2)),
        }
    }

    fn emit_pointer_index_expr_to_a(&mut self, expr: &SemExpr) -> Result<bool, String> {
        let Some(indexed) = self.classifier().pointer_index_expr(expr)? else {
            return Ok(false);
        };
        let pointee_width = indexed.element_width;
        if pointee_width != 1 {
            return Err("only byte pointer indexed reads are supported".to_string());
        }
        self.materialize_pointer_index_address(indexed, NativeAddressDestination::ArrayAddr)?;
        self.materialize_array_addr_element_to_a();
        Ok(true)
    }

    fn emit_pointer_index_expr_to_target(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        let Some(indexed) = self.classifier().pointer_index_expr(expr)? else {
            return Ok(false);
        };
        let pointee_width = indexed.element_width;
        if pointee_width != target.width {
            return Err("pointer indexed read width mismatch".to_string());
        }
        if pointee_width != 2 {
            return Ok(false);
        }
        self.materialize_pointer_index_address(indexed, NativeAddressDestination::ArrayAddr)?;
        self.materialize_array_addr_element_to_target(target)?;
        Ok(true)
    }

    fn emit_machine_block(&mut self, items: &[MachineItem], span: Span) -> Result<(), String> {
        let mut index = 0usize;
        let mut pending_operand_bytes = 0u8;
        while index < items.len() {
            match &items[index] {
                MachineItem::Number(number) => {
                    let value = number
                        .value
                        .ok_or_else(|| "machine block number is out of range".to_string())?;
                    self.emit_machine_number(value, &mut pending_operand_bytes);
                }
                MachineItem::StringLiteral(value) => {
                    let literal_address = self.current_address()?;
                    pending_operand_bytes = 0;
                    let literal = string_literal_storage_bytes(value)
                        .map_err(|err| format!("machine block string literal: {err}"))?;
                    self.emit_raw_bytes(literal);
                    self.emit_raw_u16_le(literal_address);
                }
                MachineItem::CharLiteral(value) => {
                    pending_operand_bytes = 0;
                    let byte = source_char_byte(*value).unwrap_or(0);
                    self.emit_raw_u8(byte);
                    self.emit_raw_u8(0x9A);
                }
                MachineItem::Raw(raw) if raw == "," => {}
                MachineItem::Raw(raw) if raw.starts_with('"') && raw.ends_with('"') => {
                    let literal_address = self.current_address()?;
                    pending_operand_bytes = 0;
                    let literal =
                        string_literal_storage_bytes(&raw[1..raw.len().saturating_sub(1)])
                            .map_err(|err| format!("machine block string literal: {err}"))?;
                    self.emit_raw_bytes(literal);
                    self.emit_raw_u16_le(literal_address);
                }
                MachineItem::Raw(raw) if raw.starts_with('\'') && raw.ends_with('\'') => {
                    pending_operand_bytes = 0;
                    let byte = raw[1..raw.len().saturating_sub(1)]
                        .chars()
                        .next()
                        .and_then(source_char_byte)
                        .unwrap_or(0);
                    self.emit_raw_u8(byte);
                    self.emit_raw_u8(0x9A);
                }
                MachineItem::Name(name) => {
                    let (offset, consumed) = machine_block_name_offset(&items[index + 1..]);
                    if offset == 0
                        && let Some(value) = self.numeric_define(name)
                    {
                        self.emit_machine_number(value, &mut pending_operand_bytes);
                    } else if let Some(slot) = self.machine_storage_slot(name) {
                        self.emit_machine_absolute(
                            slot.address.wrapping_add(offset),
                            &mut pending_operand_bytes,
                        );
                        index += consumed;
                    } else if offset == 0 {
                        self.emit_machine_routine_address(name, &mut pending_operand_bytes, span)?;
                    } else {
                        return Err(format!(
                            "machine block offset is not supported for `{name}`"
                        ));
                    }
                }
                MachineItem::AddressByte { selector, name } => {
                    self.emit_machine_symbol_byte(
                        *selector,
                        name,
                        &mut pending_operand_bytes,
                        span,
                    )?;
                }
                MachineItem::AddressExpr(expr) => {
                    self.emit_machine_address_expr(expr, &mut pending_operand_bytes, span)?;
                }
                MachineItem::Raw(_) => {
                    return Err("machine block item is not supported".to_string());
                }
            }
            index += 1;
        }
        self.y_known_zero = false;
        Ok(())
    }

    fn numeric_define(&self, name: &str) -> Option<u16> {
        let normalized = normalize_name(name);
        for module in &self.model.program.modules {
            for item in &module.items {
                if let SemItem::Define(define) = item
                    && normalize_name(&define.symbol.name) == normalized
                    && let Some(value) = parse_initializer_number_text(&define.value)
                {
                    return Some(value);
                }
            }
        }
        if let Some(routine_id) = self.current_routine
            && let Some(routine) = self
                .model
                .routines
                .iter()
                .find(|routine| routine.routine.symbol.id == routine_id)
        {
            for stmt in &routine.routine.body {
                if let SemStmt::Define(define) = stmt
                    && normalize_name(&define.symbol.name) == normalized
                    && let Some(value) = parse_initializer_number_text(&define.value)
                {
                    return Some(value);
                }
            }
        }
        None
    }

    fn machine_define_items(
        &self,
        symbol: &SemSymbolRef,
    ) -> Result<Option<Vec<MachineItem>>, String> {
        let Some(value) = self.machine_define_value(symbol) else {
            return Ok(None);
        };
        let tokens = crate::lexer::tokenize(value).map_err(|diagnostics| {
            format!(
                "define `{}` machine body could not be tokenized: {}",
                symbol.name,
                diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.message.as_str())
                    .unwrap_or("unknown lexer error")
            )
        })?;
        crate::parser::parse_machine_items(&tokens)
            .map(Some)
            .map_err(|diagnostics| {
                format!(
                    "define `{}` machine body could not be parsed: {}",
                    symbol.name,
                    diagnostics
                        .first()
                        .map(|diagnostic| diagnostic.message.as_str())
                        .unwrap_or("unknown parser error")
                )
            })
    }

    fn machine_define_value<'s>(&'s self, symbol: &SemSymbolRef) -> Option<&'s str> {
        if let Some(routine_id) = self.current_routine
            && let Some(routine) = self
                .model
                .routines
                .iter()
                .find(|routine| routine.routine.symbol.id == routine_id)
        {
            for stmt in &routine.routine.body {
                if let SemStmt::Define(define) = stmt
                    && define.symbol.id == symbol.id
                {
                    return Some(define.value.as_str());
                }
            }
        }

        for module in &self.model.program.modules {
            for item in &module.items {
                if let SemItem::Define(define) = item
                    && define.symbol.id == symbol.id
                {
                    return Some(define.value.as_str());
                }
            }
        }

        None
    }

    fn machine_storage_slot(&self, name: &str) -> Option<NativeStorageSlot> {
        let normalized = normalize_name(name);
        if let Some(routine_id) = self.current_routine
            && let Some(routine) = self
                .model
                .routines
                .iter()
                .find(|routine| routine.routine.symbol.id == routine_id)
        {
            for param in &routine.routine.params {
                if normalize_name(&param.symbol.name) == normalized {
                    return self.storage.get(&param.symbol.id).cloned();
                }
            }
            for local in &routine.routine.locals {
                if normalize_name(&local.symbol.name) == normalized {
                    return self.storage.get(&local.symbol.id).cloned();
                }
            }
        }

        for group in &self.model.declaration_groups {
            if !matches!(group.scope, SemIrDeclarationScope::Module(_)) {
                continue;
            }
            for declaration in &group.declarations {
                if normalize_name(&declaration.symbol.name) == normalized {
                    return self.storage.get(&declaration.symbol.id).cloned();
                }
            }
        }
        None
    }

    fn machine_caret_symbol_value(&self, name: &str) -> Option<u16> {
        let normalized = normalize_name(name);
        if let Some(routine_id) = self.current_routine
            && let Some(routine) = self
                .model
                .routines
                .iter()
                .find(|routine| routine.routine.symbol.id == routine_id)
        {
            for param in &routine.routine.params {
                if normalize_name(&param.symbol.name) == normalized {
                    return self.machine_caret_values.get(&param.symbol.id).copied();
                }
            }
            for local in &routine.routine.locals {
                if normalize_name(&local.symbol.name) == normalized {
                    return self.machine_caret_values.get(&local.symbol.id).copied();
                }
            }
        }

        for group in &self.model.declaration_groups {
            if !matches!(group.scope, SemIrDeclarationScope::Module(_)) {
                continue;
            }
            for declaration in &group.declarations {
                if normalize_name(&declaration.symbol.name) == normalized {
                    return self
                        .machine_caret_values
                        .get(&declaration.symbol.id)
                        .copied();
                }
            }
        }
        None
    }

    fn machine_routine(&self, name: &str) -> Option<&SemRoutine> {
        let normalized = normalize_name(name);
        self.model
            .routines
            .iter()
            .find(|routine| normalize_name(&routine.routine.symbol.name) == normalized)
            .map(|routine| routine.routine)
    }

    fn emit_machine_routine_address(
        &mut self,
        name: &str,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) -> Result<(), String> {
        let routine = self
            .machine_routine(name)
            .ok_or_else(|| format!("unknown machine block symbol `{name}`"))?;
        if let Some(address) = self.machine_routine_absolute_address(routine) {
            self.emit_machine_absolute(address, pending_operand_bytes);
            return Ok(());
        }
        let label = self
            .routine_labels
            .get(&routine.symbol.id)
            .cloned()
            .ok_or_else(|| format!("routine `{}` has no native label", routine.symbol.name))?;
        self.emit_machine_absolute_label(label, pending_operand_bytes, span);
        Ok(())
    }

    fn emit_machine_symbol_byte(
        &mut self,
        selector: AddressByteSelector,
        name: &str,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) -> Result<(), String> {
        if let Some(value) = self.numeric_define(name) {
            self.emit_machine_address_byte_value(selector, value);
        } else if let Some(slot) = self.machine_storage_slot(name) {
            self.emit_machine_address_byte_value(selector, slot.address);
        } else if let Some(routine) = self.machine_routine(name) {
            if let Some(address) = self.machine_routine_absolute_address(routine) {
                self.emit_machine_address_byte_value(selector, address);
            } else {
                let label = self
                    .routine_labels
                    .get(&routine.symbol.id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("routine `{}` has no native label", routine.symbol.name)
                    })?;
                match selector {
                    AddressByteSelector::Low => self.emit_raw_u8_label_low(label, span),
                    AddressByteSelector::High => self.emit_raw_u8_label_high(label, span),
                }
            }
        } else {
            return Err(format!("unknown machine block symbol `{name}`"));
        }
        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
        Ok(())
    }

    fn emit_machine_address_expr(
        &mut self,
        expr: &MachineAddressExpr,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) -> Result<(), String> {
        let offset = self.machine_address_expr_offset(expr)?;
        match &expr.atom {
            MachineAddressAtom::Number(number) => {
                let value = native_machine_number_with_offset(number, offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
            }
            MachineAddressAtom::Name(name) => {
                self.emit_machine_named_address_expr(
                    name,
                    expr,
                    offset,
                    pending_operand_bytes,
                    span,
                )?;
            }
            MachineAddressAtom::Current => {
                let value =
                    native_machine_apply_offset(self.current_address()?, offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
            }
        }
        Ok(())
    }

    fn machine_address_expr_offset(&self, expr: &MachineAddressExpr) -> Result<i32, String> {
        let mut offset = expr.offset;
        if let Some((negative, name)) = machine_address_symbolic_offset(&expr.text) {
            let Some(value) = self.numeric_define(name) else {
                return Err(format!(
                    "machine block item `{}` references unknown numeric define `{name}`",
                    expr.text
                ));
            };
            let value = i32::from(value);
            offset = offset.wrapping_add(if negative { -value } else { value });
        }
        Ok(offset)
    }

    fn emit_machine_named_address_expr(
        &mut self,
        name: &str,
        expr: &MachineAddressExpr,
        offset: i32,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) -> Result<(), String> {
        if native_machine_address_expr_uses_caret(expr) {
            let Some(value) = self.machine_caret_symbol_value(name) else {
                return Err(format!(
                    "machine block item `{}` cannot be resolved to a compile-time pointer value",
                    expr.text
                ));
            };
            let value = native_machine_apply_offset(value, offset, &expr.text)?;
            self.emit_machine_resolved_address_value(value, expr.selector, pending_operand_bytes);
        } else if let Some(value) = self.numeric_define(name) {
            let value = native_machine_apply_offset(value, offset, &expr.text)?;
            self.emit_machine_resolved_address_value(value, expr.selector, pending_operand_bytes);
        } else if let Some(slot) = self.machine_storage_slot(name) {
            let value = native_machine_apply_offset(slot.address, offset, &expr.text)?;
            self.emit_machine_resolved_address_value(value, expr.selector, pending_operand_bytes);
        } else if let Some(routine) = self.machine_routine(name) {
            if let Some(address) = self.machine_routine_absolute_address(routine) {
                let value = native_machine_apply_offset(address, offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
            } else if offset == 0 {
                let label = self
                    .routine_labels
                    .get(&routine.symbol.id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("routine `{}` has no native label", routine.symbol.name)
                    })?;
                match expr.selector {
                    Some(AddressByteSelector::Low) => {
                        self.emit_raw_u8_label_low(label, span);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    }
                    Some(AddressByteSelector::High) => {
                        self.emit_raw_u8_label_high(label, span);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    }
                    None => self.emit_machine_absolute_label(label, pending_operand_bytes, span),
                }
            } else {
                return Err(format!(
                    "machine block item `{}` with offset is not relocatable yet",
                    expr.text
                ));
            }
        } else {
            return Err(format!("unknown machine block symbol `{name}`"));
        }
        Ok(())
    }

    fn emit_machine_resolved_address_value(
        &mut self,
        value: u16,
        selector: Option<AddressByteSelector>,
        pending_operand_bytes: &mut u8,
    ) {
        match selector {
            Some(AddressByteSelector::Low) => {
                self.emit_raw_u8((value & 0x00FF) as u8);
                *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
            }
            Some(AddressByteSelector::High) => {
                self.emit_raw_u8((value >> 8) as u8);
                *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
            }
            None if value <= 0xFF => self.emit_machine_number(value, pending_operand_bytes),
            None => self.emit_machine_absolute(value, pending_operand_bytes),
        }
    }

    fn machine_routine_absolute_address(&self, routine: &SemRoutine) -> Option<u16> {
        let address = routine.system_address.as_ref()?;
        if is_current_location_expr(address) {
            return None;
        }
        self.constant_word(address)
    }

    fn emit_machine_address_byte_value(&mut self, selector: AddressByteSelector, value: u16) {
        let byte = match selector {
            AddressByteSelector::Low => (value & 0x00FF) as u8,
            AddressByteSelector::High => (value >> 8) as u8,
        };
        self.emit_raw_u8(byte);
    }

    fn emit_machine_number(&mut self, value: u16, pending_operand_bytes: &mut u8) {
        if value > 0xFF {
            self.emit_raw_u16_le(value);
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
            return;
        }

        let byte = Immediate::new(value).low();
        self.emit_raw_u8(byte);
        if *pending_operand_bytes > 0 {
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
        } else {
            *pending_operand_bytes = native_machine_opcode_operand_bytes(byte);
        }
    }

    fn emit_machine_absolute(&mut self, address: u16, pending_operand_bytes: &mut u8) {
        if *pending_operand_bytes == 1 {
            self.emit_raw_u8((address & 0x00FF) as u8);
            *pending_operand_bytes = 0;
        } else {
            self.emit_raw_u16_le(address);
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
        }
    }

    fn emit_machine_absolute_label(
        &mut self,
        label: String,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) {
        self.emit_raw_u16_label(label, span);
        *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
    }

    fn emit_byte_expr_to_a(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.emit_byte_expr_to_a(expr);
        }
        if let Some(value) = literal_byte(expr) {
            self.emit_lda_imm(value);
            return Ok(());
        }
        if self.emit_pointer_index_expr_to_a(expr)? {
            return Ok(());
        }
        if self.emit_array_index_expr_to_a(expr)? {
            return Ok(());
        }
        if self.emit_expr_to_a(expr)? {
            return Ok(());
        }
        let slot = self.classifier().required_addressable_slot(expr)?;
        if slot.width != 1 {
            return Err(format!(
                "only byte expressions are supported (got {} byte source: {})",
                slot.width,
                native_expr_debug_name(expr)
            ));
        }
        self.emit_lda_addr(slot.address);
        Ok(())
    }

    fn emit_byte_expr_to_x(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.emit_byte_expr_to_x(expr);
        }
        if let Some(value) = literal_byte(expr) {
            self.emit_ldx_imm(value);
            return Ok(());
        }
        if self.emit_expr_to_a(expr)? {
            self.emit_tax();
            return Ok(());
        }
        let slot = self.classifier().required_addressable_slot(expr)?;
        if slot.width != 1 {
            return Err(format!(
                "only byte expressions are supported (got {} byte source: {})",
                slot.width,
                native_expr_debug_name(expr)
            ));
        }
        self.emit_ldx_addr(slot.address);
        Ok(())
    }

    fn emit_byte_expr_to_y(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.emit_byte_expr_to_y(expr);
        }
        if let Some(value) = literal_byte(expr) {
            self.emit_ldy_imm(value);
            return Ok(());
        }
        if self.emit_expr_to_a(expr)? {
            self.emit_tay();
            return Ok(());
        }
        let slot = self.classifier().required_addressable_slot(expr)?;
        if slot.width != 1 {
            return Err(format!(
                "only byte expressions are supported (got {} byte source: {})",
                slot.width,
                native_expr_debug_name(expr)
            ));
        }
        self.emit_ldy_addr(slot.address);
        Ok(())
    }

    fn emit_if(&mut self, branches: &[SemIfBranch], else_body: &[SemStmt]) -> Result<(), String> {
        if branches.is_empty() {
            return self.emit_statement_list(else_body);
        }
        let end_label = self.next_label("if_end");

        for (index, branch) in branches.iter().enumerate() {
            let then_label = self.next_label("if_then");
            let false_label = if index + 1 == branches.len() {
                self.next_label("if_else")
            } else {
                self.next_label("if_next")
            };

            self.emit_condition_branch(&branch.condition, &then_label, &false_label)?;
            self.bind_label(&then_label, branch.condition.span)?;
            self.emit_statement_list(&branch.body)?;
            self.emit_jmp_label(end_label.clone(), branch.condition.span);
            self.bind_label(&false_label, branch.condition.span)?;
        }
        self.emit_statement_list(else_body)?;
        self.bind_label(&end_label, branches[0].condition.span)?;
        Ok(())
    }

    fn emit_while(&mut self, condition: &SemCondition, body: &[SemStmt]) -> Result<(), String> {
        let start_label = self.next_label("while_start");
        let body_label = self.next_label("while_body");
        let end_label = self.next_label("while_end");

        self.bind_label(&start_label, condition.span)?;
        self.emit_condition_branch(condition, &body_label, &end_label)?;
        self.bind_label(&body_label, condition.span)?;
        self.exit_labels.push(end_label.clone());
        self.emit_statement_list(body)?;
        self.exit_labels.pop();
        self.emit_jmp_label(start_label.clone(), condition.span);
        self.bind_label(&end_label, condition.span)?;
        Ok(())
    }

    fn emit_do_until(
        &mut self,
        body: &[SemStmt],
        condition: Option<&SemCondition>,
        span: Span,
    ) -> Result<(), String> {
        let start_label = self.next_label("do_start");
        let end_label = self.next_label("do_end");

        self.bind_label(&start_label, span)?;
        self.exit_labels.push(end_label.clone());
        self.emit_statement_list(body)?;
        self.exit_labels.pop();
        if let Some(condition) = condition {
            self.emit_condition_branch(condition, &end_label, &start_label)?;
        } else {
            self.emit_jmp_label(start_label.clone(), span);
        }
        self.bind_label(&end_label, span)?;
        Ok(())
    }

    fn emit_for(
        &mut self,
        target: &SemLValue,
        start: &SemExpr,
        end: &SemExpr,
        step: Option<&SemExpr>,
        body: &[SemStmt],
        span: Span,
    ) -> Result<(), String> {
        let step = self.native_for_step(step)?;

        self.emit_assignment(target, start)?;
        let target = self.classifier().required_lvalue_slot(target)?;
        if !matches!(target.width, 1 | 2) {
            return Err(format!(
                "only byte/card FOR targets are supported (got {} bytes)",
                target.width
            ));
        }

        let start_label = self.next_label("for_start");
        let body_label = self.next_label("for_body");
        let end_label = self.next_label("for_end");

        self.bind_label(&start_label, span)?;
        self.emit_for_branch_if_not_after_end(&target, end, step, &body_label, &end_label, span)?;
        self.bind_label(&body_label, span)?;
        self.exit_labels.push(end_label.clone());
        self.emit_statement_list(body)?;
        self.exit_labels.pop();
        self.emit_for_increment(&target, step, span)?;
        self.emit_jmp_label(start_label.clone(), span);
        self.bind_label(&end_label, span)?;
        Ok(())
    }

    fn emit_for_branch_if_not_after_end(
        &mut self,
        target: &NativeResolvedSlot,
        end: &SemExpr,
        step: NativeForStep,
        body_label: &str,
        end_label: &str,
        span: Span,
    ) -> Result<(), String> {
        if matches!(step, NativeForStep::Down(_)) {
            return self
                .emit_for_branch_if_not_before_end(target, end, body_label, end_label, span);
        }
        match target.width {
            1 => {
                self.emit_cmp_target_byte_to_end(target.address, end, 0)?;
                self.emit_bcc_label(body_label, span);
                self.emit_beq_label(body_label, span);
                self.emit_jmp_label(end_label, span);
            }
            2 => {
                self.emit_cmp_target_byte_to_end(target.address + 1, end, 1)?;
                self.emit_bcc_label(body_label, span);
                self.emit_bne_label(end_label, span);
                self.emit_cmp_target_byte_to_end(target.address, end, 0)?;
                self.emit_bcc_label(body_label, span);
                self.emit_beq_label(body_label, span);
                self.emit_jmp_label(end_label, span);
            }
            _ => unreachable!("FOR target width checked before comparison lowering"),
        }
        Ok(())
    }

    fn emit_for_branch_if_not_before_end(
        &mut self,
        target: &NativeResolvedSlot,
        end: &SemExpr,
        body_label: &str,
        end_label: &str,
        span: Span,
    ) -> Result<(), String> {
        match target.width {
            1 => {
                self.emit_cmp_target_byte_to_end(target.address, end, 0)?;
                self.emit_bcs_label(body_label, span);
                self.emit_jmp_label(end_label, span);
            }
            2 => {
                self.emit_cmp_target_byte_to_end(target.address + 1, end, 1)?;
                self.emit_bcc_label(end_label, span);
                self.emit_bne_label(body_label, span);
                self.emit_cmp_target_byte_to_end(target.address, end, 0)?;
                self.emit_bcs_label(body_label, span);
                self.emit_jmp_label(end_label, span);
            }
            _ => unreachable!("FOR target width checked before comparison lowering"),
        }
        Ok(())
    }

    fn emit_cmp_target_byte_to_end(
        &mut self,
        target_address: u16,
        end: &SemExpr,
        byte_index: u16,
    ) -> Result<(), String> {
        if let Some(source) = self.classifier().compare_byte_source(end, byte_index)? {
            self.emit_lda_addr(target_address);
            self.emit_cmp_byte_source(source)?;
            return Ok(());
        }
        if byte_index == 0 && self.materialize_byte_value_address_to_array_addr(end)? {
            self.emit_lda_addr(target_address);
            self.ensure_y_zero();
            self.emit_cmp_array_addr_indirect_y();
            return Ok(());
        }
        if byte_index == 0 && expr_width(end) == Some(1) {
            self.emit_byte_expr_to_a(end)?;
            self.emit_sta_afcur();
            self.emit_lda_addr(target_address);
            self.emit_cmp_afcur();
            return Ok(());
        }
        Err("only simple FOR bounds are supported".to_string())
    }

    fn emit_for_increment(
        &mut self,
        target: &NativeResolvedSlot,
        step: NativeForStep,
        span: Span,
    ) -> Result<(), String> {
        match (target.width, step) {
            (1, NativeForStep::Up(1)) => self.emit_inc_addr(target.address),
            (1, NativeForStep::Down(1)) => self.emit_dec_addr(target.address),
            (1, NativeForStep::Up(step)) => {
                self.emit_byte_for_step(target.address, step, true)?;
            }
            (1, NativeForStep::Down(step)) => {
                self.emit_byte_for_step(target.address, step, false)?;
            }
            (2, NativeForStep::Up(step)) => self.emit_word_for_step(target.address, step, true),
            (2, NativeForStep::Down(step)) => self.emit_word_for_step(target.address, step, false),
            _ => {
                return Err(format!(
                    "only byte/card FOR targets are supported at {}..{}",
                    span.start, span.end
                ));
            }
        }
        Ok(())
    }

    fn emit_byte_for_step(&mut self, address: u16, step: u16, add: bool) -> Result<(), String> {
        let step =
            u8::try_from(step).map_err(|_| "byte FOR step must fit in one byte".to_string())?;
        if add {
            self.emit_clc();
            self.emit_lda_addr(address);
            self.emit_adc_imm(step);
            self.emit_sta_addr(address);
        } else {
            self.emit_sec();
            self.emit_lda_addr(address);
            self.emit_sbc_imm(step);
            self.emit_sta_addr(address);
        }
        Ok(())
    }

    fn emit_word_for_step(&mut self, address: u16, step: u16, add: bool) {
        if add {
            self.emit_clc();
            self.emit_lda_addr(address);
            self.emit_adc_imm((step & 0x00FF) as u8);
            self.emit_sta_addr(address);
            self.emit_lda_addr(address + 1);
            self.emit_adc_imm((step >> 8) as u8);
            self.emit_sta_addr(address + 1);
        } else {
            self.emit_sec();
            self.emit_lda_addr(address);
            self.emit_sbc_imm((step & 0x00FF) as u8);
            self.emit_sta_addr(address);
            self.emit_lda_addr(address + 1);
            self.emit_sbc_imm((step >> 8) as u8);
            self.emit_sta_addr(address + 1);
        }
    }

    fn native_for_step(&self, step: Option<&SemExpr>) -> Result<NativeForStep, String> {
        let Some(step) = step else {
            return Ok(NativeForStep::Up(1));
        };
        let direction = match &step.kind {
            SemExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            } => NativeForStep::Down(
                self.constant_word(expr)
                    .filter(|amount| *amount > 0)
                    .ok_or_else(|| "FOR step must be a non-zero constant".to_string())?,
            ),
            _ => NativeForStep::Up(
                self.constant_word(step)
                    .filter(|amount| *amount > 0)
                    .ok_or_else(|| "FOR step must be a non-zero constant".to_string())?,
            ),
        };
        Ok(direction)
    }

    fn emit_exit(&mut self, span: Span) -> Result<(), String> {
        let Some(label) = self.exit_labels.last() else {
            return Err("EXIT is not inside a loop".to_string());
        };
        self.emit_jmp_label(label.clone(), span);
        Ok(())
    }

    fn emit_statement_list(&mut self, statements: &[SemStmt]) -> Result<(), String> {
        for stmt in statements {
            self.emit_statement(stmt)?;
        }
        Ok(())
    }

    fn emit_condition_branch(
        &mut self,
        condition: &SemCondition,
        true_label: &str,
        false_label: &str,
    ) -> Result<(), String> {
        match condition.kind {
            SemConditionKind::ConstantTrue => {
                self.emit_jmp_label(true_label, condition.span);
                return Ok(());
            }
            SemConditionKind::ConstantFalse => {
                self.emit_jmp_label(false_label, condition.span);
                return Ok(());
            }
            SemConditionKind::NonZeroValue => {
                if sem_condition_shaped_expr(&condition.expr) {
                    return self.emit_condition_expr_branch(
                        &condition.expr,
                        condition.span,
                        true_label,
                        false_label,
                    );
                }
                return self.emit_nonzero_expr_branch(
                    &condition.expr,
                    condition.span,
                    true_label,
                    false_label,
                );
            }
            SemConditionKind::Compare | SemConditionKind::Logical => {}
            SemConditionKind::Error | SemConditionKind::Unknown => {
                return Err("unsupported control-flow condition".to_string());
            }
        }
        self.emit_condition_expr_branch(&condition.expr, condition.span, true_label, false_label)
    }

    fn emit_nonzero_expr_branch(
        &mut self,
        expr: &SemExpr,
        span: Span,
        true_label: &str,
        false_label: &str,
    ) -> Result<(), String> {
        if self.emit_word_nonzero_condition(expr, true_label, false_label, span)? {
            return Ok(());
        }
        self.emit_condition_value_to_a(expr)?;
        self.emit_bne_label(true_label, span);
        self.emit_jmp_label(false_label, span);
        Ok(())
    }

    fn emit_condition_expr_branch(
        &mut self,
        expr: &SemExpr,
        span: Span,
        true_label: &str,
        false_label: &str,
    ) -> Result<(), String> {
        let SemExprKind::Binary { op, left, right } = &expr.kind else {
            if self.emit_word_nonzero_condition(expr, true_label, false_label, span)? {
                return Ok(());
            }
            self.emit_condition_value_to_a(expr)?;
            self.emit_bne_label(true_label, span);
            self.emit_jmp_label(false_label, span);
            return Ok(());
        };
        match op {
            BinaryOp::Or => {
                if !sem_logical_binary_condition(left, right) {
                    return self.emit_nonzero_expr_branch(expr, span, true_label, false_label);
                }
                let right_label = self.next_label("cond_or_rhs");
                self.emit_condition_expr_branch(left, span, true_label, &right_label)?;
                self.bind_label(&right_label, span)?;
                self.emit_condition_expr_branch(right, span, true_label, false_label)?;
            }
            BinaryOp::And => {
                if !sem_logical_binary_condition(left, right) {
                    return self.emit_nonzero_expr_branch(expr, span, true_label, false_label);
                }
                let right_label = self.next_label("cond_and_rhs");
                self.emit_condition_expr_branch(left, span, &right_label, false_label)?;
                self.bind_label(&right_label, span)?;
                self.emit_condition_expr_branch(right, span, true_label, false_label)?;
            }
            BinaryOp::Eq => {
                if self.emit_word_equality_condition(
                    left,
                    right,
                    true,
                    true_label,
                    false_label,
                    span,
                )? {
                    return Ok(());
                }
                self.emit_condition_value_to_a(left)?;
                self.emit_condition_eor_operand(right)?;
                self.emit_beq_label(true_label, span);
                self.emit_jmp_label(false_label, span);
            }
            BinaryOp::Ne => {
                if self.emit_word_equality_condition(
                    left,
                    right,
                    false,
                    true_label,
                    false_label,
                    span,
                )? {
                    return Ok(());
                }
                self.emit_condition_value_to_a(left)?;
                self.emit_condition_eor_operand(right)?;
                self.emit_bne_label(true_label, span);
                self.emit_jmp_label(false_label, span);
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                if self.emit_signed_word_zero_ordering_condition(
                    *op,
                    left,
                    right,
                    true_label,
                    false_label,
                    span,
                )? {
                    return Ok(());
                }
                if self.emit_unsigned_word_ordering_condition(
                    *op,
                    left,
                    right,
                    true_label,
                    false_label,
                    span,
                )? {
                    return Ok(());
                }
                self.emit_condition_value_to_a(left)?;
                self.emit_condition_cmp_operand(right)?;
                match op {
                    BinaryOp::Lt => {
                        self.emit_bcc_label(true_label, span);
                        self.emit_jmp_label(false_label, span);
                    }
                    BinaryOp::Le => {
                        self.emit_bcc_label(true_label, span);
                        self.emit_beq_label(true_label, span);
                        self.emit_jmp_label(false_label, span);
                    }
                    BinaryOp::Gt => {
                        self.emit_bcc_label(false_label, span);
                        self.emit_beq_label(false_label, span);
                        self.emit_jmp_label(true_label, span);
                    }
                    BinaryOp::Ge => {
                        self.emit_bcs_label(true_label, span);
                        self.emit_jmp_label(false_label, span);
                    }
                    _ => unreachable!("condition operator already matched"),
                }
            }
            _ => return Err("only =, #, and byte ordering conditions are supported".to_string()),
        }
        Ok(())
    }

    fn emit_word_equality_condition(
        &mut self,
        left: &SemExpr,
        right: &SemExpr,
        is_equal: bool,
        true_label: &str,
        false_label: &str,
        span: Span,
    ) -> Result<bool, String> {
        if !self.condition_operand_requires_word_compare(left)
            && !self.condition_operand_requires_word_compare(right)
        {
            return Ok(false);
        }

        let right = self.word_condition_operand_sources(right, native_args_offset_slot(2, 2)?)?;
        let left = self.word_condition_operand_sources(left, native_args_slot(2))?;

        self.materialize_byte_source_to_register(left.low, NativeByteRegister::A)?;
        self.emit_eor_byte_source(right.low)?;
        if is_equal {
            self.emit_bne_label(false_label, span);
        } else {
            self.emit_bne_label(true_label, span);
        }

        self.materialize_byte_source_to_register(left.high, NativeByteRegister::A)?;
        self.emit_eor_byte_source(right.high)?;
        if is_equal {
            self.emit_beq_label(true_label, span);
            self.emit_jmp_label(false_label, span);
        } else {
            self.emit_bne_label(true_label, span);
            self.emit_jmp_label(false_label, span);
        }
        Ok(true)
    }

    fn emit_unsigned_word_ordering_condition(
        &mut self,
        op: BinaryOp,
        left: &SemExpr,
        right: &SemExpr,
        true_label: &str,
        false_label: &str,
        span: Span,
    ) -> Result<bool, String> {
        if !matches!(
            op,
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) || !self.condition_operand_requires_word_compare(left)
            && !self.condition_operand_requires_word_compare(right)
        {
            return Ok(false);
        }
        if self.condition_operand_is_signed_word(left)
            || self.condition_operand_is_signed_word(right)
        {
            return Ok(false);
        }

        let right = self.word_condition_operand_sources(right, native_args_offset_slot(2, 2)?)?;
        let left = self.word_condition_operand_sources(left, native_args_slot(2))?;

        self.materialize_byte_source_to_register(left.high, NativeByteRegister::A)?;
        self.emit_cmp_byte_source(right.high)?;
        match op {
            BinaryOp::Lt | BinaryOp::Le => {
                self.emit_bcc_label(true_label, span);
                self.emit_bne_label(false_label, span);
                self.materialize_byte_source_to_register(left.low, NativeByteRegister::A)?;
                self.emit_cmp_byte_source(right.low)?;
                self.emit_bcc_label(true_label, span);
                if op == BinaryOp::Le {
                    self.emit_beq_label(true_label, span);
                }
                self.emit_jmp_label(false_label, span);
            }
            BinaryOp::Gt | BinaryOp::Ge => {
                self.emit_bcc_label(false_label, span);
                self.emit_bne_label(true_label, span);
                self.materialize_byte_source_to_register(left.low, NativeByteRegister::A)?;
                self.emit_cmp_byte_source(right.low)?;
                if op == BinaryOp::Gt {
                    self.emit_bcc_label(false_label, span);
                    self.emit_beq_label(false_label, span);
                    self.emit_jmp_label(true_label, span);
                } else {
                    self.emit_bcs_label(true_label, span);
                    self.emit_jmp_label(false_label, span);
                }
            }
            _ => unreachable!("word ordering operator already matched"),
        }
        Ok(true)
    }

    fn emit_signed_word_zero_ordering_condition(
        &mut self,
        op: BinaryOp,
        left: &SemExpr,
        right: &SemExpr,
        true_label: &str,
        false_label: &str,
        span: Span,
    ) -> Result<bool, String> {
        if !matches!(
            op,
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) {
            return Ok(false);
        }

        if literal_word(right) == Some(0) && self.condition_operand_is_signed_word(left) {
            let left = self.word_condition_operand_sources(left, native_args_slot(2))?;
            self.emit_signed_word_source_zero_ordering(op, left, true_label, false_label, span)?;
            return Ok(true);
        }

        if literal_word(left) == Some(0) && self.condition_operand_is_signed_word(right) {
            let right = self.word_condition_operand_sources(right, native_args_slot(2))?;
            let flipped = match op {
                BinaryOp::Lt => BinaryOp::Gt,
                BinaryOp::Le => BinaryOp::Ge,
                BinaryOp::Gt => BinaryOp::Lt,
                BinaryOp::Ge => BinaryOp::Le,
                _ => unreachable!("signed word zero operator checked by caller"),
            };
            self.emit_signed_word_source_zero_ordering(
                flipped,
                right,
                true_label,
                false_label,
                span,
            )?;
            return Ok(true);
        }

        Ok(false)
    }

    fn emit_signed_word_source_zero_ordering(
        &mut self,
        op: BinaryOp,
        source: NativeWordSource,
        true_label: &str,
        false_label: &str,
        span: Span,
    ) -> Result<(), String> {
        self.materialize_byte_source_to_register(source.high, NativeByteRegister::A)?;
        self.emit_and_imm(0x80);
        match op {
            BinaryOp::Lt => {
                self.emit_bne_label(true_label, span);
                self.emit_jmp_label(false_label, span);
            }
            BinaryOp::Ge => {
                self.emit_bne_label(false_label, span);
                self.emit_jmp_label(true_label, span);
            }
            BinaryOp::Gt => {
                self.emit_bne_label(false_label, span);
                self.emit_signed_word_nonzero_branch(source, true_label, false_label, span)?;
            }
            BinaryOp::Le => {
                self.emit_bne_label(true_label, span);
                self.emit_signed_word_nonzero_branch(source, false_label, true_label, span)?;
            }
            _ => unreachable!("signed word zero operator checked by caller"),
        }
        Ok(())
    }

    fn emit_signed_word_nonzero_branch(
        &mut self,
        source: NativeWordSource,
        nonzero_label: &str,
        zero_label: &str,
        span: Span,
    ) -> Result<(), String> {
        self.materialize_byte_source_to_register(source.low, NativeByteRegister::A)?;
        self.emit_bne_label(nonzero_label, span);
        self.materialize_byte_source_to_register(source.high, NativeByteRegister::A)?;
        self.emit_bne_label(nonzero_label, span);
        self.emit_jmp_label(zero_label, span);
        Ok(())
    }

    fn word_condition_operand_sources(
        &mut self,
        expr: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<NativeWordSource, String> {
        if let Some(low) = self.classifier().compare_byte_source(expr, 0)?
            && let Some(high) = self.classifier().compare_byte_source(expr, 1)?
        {
            return Ok(NativeWordSource { low, high });
        }
        if !self.materialize_value_to_target(expr, target.clone())?
            && !self.emit_expr_to_target(expr, target.clone())?
        {
            return Err(format!(
                "only word ordering operands are supported ({})",
                native_expr_debug_name(expr)
            ));
        }
        Ok(NativeWordSource {
            low: NativeByteSource::Storage {
                address: target.address,
            },
            high: NativeByteSource::Storage {
                address: target.address + 1,
            },
        })
    }

    fn condition_operand_is_signed_word(&self, expr: &SemExpr) -> bool {
        expr_width(expr) == Some(2)
            && expr.type_facts().signedness == Some(ScalarSignedness::Signed)
    }

    fn emit_word_nonzero_condition(
        &mut self,
        expr: &SemExpr,
        true_label: &str,
        false_label: &str,
        span: Span,
    ) -> Result<bool, String> {
        if !self.condition_operand_requires_word_compare(expr) {
            return Ok(false);
        }
        if !self.materialize_value_to_target(expr, native_args_slot(2))?
            && !self.emit_expr_to_target(expr, native_args_slot(2))?
        {
            return Err("only simple word nonzero conditions are supported".to_string());
        }
        self.emit_lda_args(0);
        self.emit_bne_label(true_label, span);
        self.emit_lda_args(1);
        self.emit_bne_label(true_label, span);
        self.emit_jmp_label(false_label, span);
        Ok(true)
    }

    fn condition_operand_requires_word_compare(&self, expr: &SemExpr) -> bool {
        match self.classifier().value_shape(expr) {
            Ok(NativeValueShape::Literal { value, .. }) => value > 0x00FF,
            Ok(NativeValueShape::Storage(slot)) => slot.width == 2,
            Ok(NativeValueShape::Address(_)) => true,
            Ok(NativeValueShape::Deref { width, .. })
            | Ok(NativeValueShape::Indexed(NativeIndexedShape {
                element_width: width,
                ..
            })) => width == 2,
            Ok(NativeValueShape::CallResult {
                width: Some(width), ..
            })
            | Ok(NativeValueShape::Computed { width: Some(width) }) => width == 2,
            Ok(NativeValueShape::CallResult { width: None, .. })
            | Ok(NativeValueShape::Computed { width: None })
            | Ok(NativeValueShape::Unsupported { .. })
            | Err(_) => false,
        }
    }

    fn emit_condition_value_to_a(&mut self, expr: &SemExpr) -> Result<(), String> {
        if self.materialize_byte_value_to_a(expr)? {
            return Ok(());
        }
        self.emit_byte_expr_to_a(expr).map_err(|reason| {
            format!(
                "only byte control-flow conditions are supported ({}: {reason})",
                native_expr_debug_name(expr),
            )
        })
    }

    fn emit_condition_eor_operand(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let Some(source) = self.classifier().compare_byte_source(expr, 0)? {
            self.emit_eor_byte_source(source)?;
            return Ok(());
        }
        self.emit_sta_element_addr();
        self.emit_condition_value_to_a(expr)?;
        self.emit_eor_element_addr();
        Ok(())
    }

    fn emit_condition_cmp_operand(&mut self, expr: &SemExpr) -> Result<(), String> {
        if let Some(source) = self.classifier().compare_byte_source(expr, 0)? {
            self.emit_cmp_byte_source(source)?;
            return Ok(());
        }
        self.emit_sta_element_addr();
        self.emit_condition_value_to_a(expr)?;
        self.emit_sta_afcur();
        self.emit_lda_element_addr();
        self.emit_cmp_afcur();
        Ok(())
    }

    fn emit_cmp_byte_source(&mut self, source: NativeByteSource) -> Result<(), String> {
        match source {
            NativeByteSource::Immediate(byte) => self.emit_cmp_imm(byte),
            NativeByteSource::Storage { address } => self.emit_cmp_addr(address),
        }
        Ok(())
    }

    fn emit_eor_byte_source(&mut self, source: NativeByteSource) -> Result<(), String> {
        match source {
            NativeByteSource::Immediate(byte) => self.emit_eor_imm(byte),
            NativeByteSource::Storage { address } => self.emit_eor_addr(address),
        }
        Ok(())
    }

    fn bind_label(&mut self, label: &str, span: Span) -> Result<(), String> {
        let result = self
            .emitter
            .bind_label(label.to_string(), span)
            .map_err(|diagnostic| diagnostic.message);
        self.y_known_zero = false;
        result
    }

    fn next_label(&mut self, prefix: &str) -> String {
        let label = format!("semir_native:{prefix}:{}", self.label_counter);
        self.label_counter += 1;
        label
    }

    fn resolved_lvalue_slot(&self, target: &SemLValue) -> Result<NativeResolvedSlot, String> {
        match &target.kind {
            SemLValueKind::Symbol(symbol) => self.resolved_symbol_slot(symbol),
            SemLValueKind::Index { base, index, .. } => self.resolved_index_slot(base, index),
            SemLValueKind::Field { base, field } => self.resolved_record_field_slot(base, field),
            _ => Err("only symbol and constant-index assignment targets are supported".to_string()),
        }
    }

    fn resolved_symbol_slot(&self, symbol: &SemSymbolRef) -> Result<NativeResolvedSlot, String> {
        let Some(slot) = self.storage.get(&symbol.id).cloned() else {
            if native_builtin_array_storage_slot(&symbol.name).is_some() {
                return Err(format!(
                    "array symbol `{}` needs an explicit index",
                    symbol.name
                ));
            }
            if let Some(slot) = native_builtin_variable_slot(&symbol.name) {
                return Ok(slot);
            }
            return Err(format!("symbol `{}` has no native storage", symbol.name));
        };
        if slot.array.is_some() && !slot_is_array_pointer_value(&slot) {
            return Err(format!(
                "array symbol `{}` needs an explicit index",
                symbol.name
            ));
        }
        Ok(NativeResolvedSlot {
            address: slot.address,
            width: slot.width,
            pointee_width: slot.pointee_width,
            record: slot.record.clone(),
        })
    }

    fn resolved_index_slot(
        &self,
        base: &SemExpr,
        index: &SemExpr,
    ) -> Result<NativeResolvedSlot, String> {
        let (symbol, slot) = self.array_slot_from_expr(base)?;
        let index = literal_word(index)
            .ok_or_else(|| format!("array `{}` index must be constant", symbol))?;
        self.resolved_array_element_slot(&symbol, slot, index)
    }

    fn resolved_record_field_slot(
        &self,
        base: &SemLValue,
        field: &SemFieldRef,
    ) -> Result<NativeResolvedSlot, String> {
        let base = self.resolved_lvalue_slot(base)?;
        if base.width == 2 && base.pointee_width.is_some() {
            return Err("record pointer fields need indirect materialization".to_string());
        }
        let field_layout = self.native_record_field_layout(&base, field)?;
        Ok(NativeResolvedSlot {
            address: base
                .address
                .checked_add(field_layout.offset)
                .ok_or_else(|| "record field address overflow".to_string())?,
            width: field_layout.width,
            pointee_width: field
                .ty
                .as_pointer()
                .and_then(|pointer| self.native_type_width(&pointer.pointee)),
            record: self.native_record_name_for_type(&field.ty),
        })
    }

    fn resolved_call_index_slot(&self, call: &SemCall) -> Result<NativeResolvedSlot, String> {
        let SemCallable::User(symbol) = &call.callee else {
            return Err("only array call-index expressions are supported".to_string());
        };
        if call.args.len() != 1 {
            return Err(format!("array `{}` needs exactly one index", symbol.name));
        }
        let slot = self
            .storage
            .get(&symbol.id)
            .cloned()
            .ok_or_else(|| format!("symbol `{}` has no native storage", symbol.name))?;
        let index = literal_word(&call.args[0])
            .ok_or_else(|| format!("array `{}` index must be constant", symbol.name))?;
        self.resolved_array_element_slot(&symbol.name, slot, index)
    }

    fn array_slot_from_expr(&self, expr: &SemExpr) -> Result<(String, NativeStorageSlot), String> {
        let symbol = self
            .classifier()
            .array_base_symbol(expr)
            .ok_or_else(|| "only symbol array bases are supported".to_string())?;
        let slot = self
            .storage
            .get(&symbol.id)
            .cloned()
            .or_else(|| native_builtin_array_storage_slot(&symbol.name))
            .ok_or_else(|| format!("symbol `{}` has no native storage", symbol.name))?;
        Ok((symbol.name.clone(), slot))
    }

    fn resolved_array_element_slot(
        &self,
        symbol_name: &str,
        slot: NativeStorageSlot,
        index: u16,
    ) -> Result<NativeResolvedSlot, String> {
        let Some(array) = slot.array else {
            return Err(format!("symbol `{symbol_name}` is not an array"));
        };
        if array.len > 0 && index >= array.len {
            return Err(format!(
                "array `{}` constant index {} is out of bounds {}",
                symbol_name, index, array.len
            ));
        }
        let offset = index
            .checked_mul(array.element_width)
            .ok_or_else(|| "array index offset overflow".to_string())?;
        Ok(NativeResolvedSlot {
            address: slot.address + offset,
            width: array.element_width,
            pointee_width: None,
            record: slot.record.clone(),
        })
    }

    fn current_address(&self) -> Result<u16, String> {
        self.model
            .origin
            .checked_add(self.emitter.position() as u16)
            .ok_or_else(|| "native output address overflow".to_string())
    }
}

#[derive(Debug)]
struct SemIrReadModel<'a> {
    program: &'a SemProgram,
    origin: u16,
    profile: CodegenProfile,
    declaration_groups: Vec<SemIrDeclarationGroup<'a>>,
    routines: Vec<SemIrRoutineView<'a>>,
    routines_by_name: HashMap<String, usize>,
    declarations_by_name: HashMap<String, usize>,
}

#[derive(Debug)]
struct SemIrDeclarationGroup<'a> {
    scope: SemIrDeclarationScope,
    span: Span,
    kind: SemIrDeclarationGroupKind,
    declarations: Vec<&'a SemDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SemIrDeclarationScope {
    Module(usize),
    Routine(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemIrDeclarationGroupKind {
    Variables,
    Type,
    Record,
}

#[derive(Debug)]
struct SemIrRoutineView<'a> {
    routine: &'a SemRoutine,
    params: Vec<SemIrParamView<'a>>,
    local_group_indexes: Vec<usize>,
    body_summary: SemIrBodySummary,
}

#[derive(Debug)]
struct SemIrParamView<'a> {
    param: &'a SemParam,
    lvalue_shape: SemIrLValueShape,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SemIrBodySummary {
    statements: usize,
    assignments: usize,
    calls: usize,
    returns: usize,
    branches: usize,
    loops: usize,
    machine_blocks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemIrLValueShape {
    Symbol,
    UnresolvedName,
    Deref,
    CallIndex,
    Index,
    Field,
}

impl<'a> SemIrReadModel<'a> {
    fn new(program: &'a SemProgram, origin: u16, profile: CodegenProfile) -> Self {
        let mut model = Self {
            program,
            origin,
            profile,
            declaration_groups: Vec::new(),
            routines: Vec::new(),
            routines_by_name: HashMap::new(),
            declarations_by_name: HashMap::new(),
        };
        model.collect_program(program);
        model
    }

    fn collect_program(&mut self, program: &'a SemProgram) {
        for (module_index, module) in program.modules.iter().enumerate() {
            self.collect_module(module_index, module);
        }
    }

    fn collect_module(&mut self, module_index: usize, module: &'a SemModule) {
        let mut index = 0usize;
        while index < module.items.len() {
            match &module.items[index] {
                SemItem::Declaration(_) => {
                    let end = self.declaration_group_end(&module.items, index);
                    self.push_declaration_group(
                        SemIrDeclarationScope::Module(module_index),
                        &module.items[index..end],
                    );
                    index = end;
                }
                SemItem::Routine(routine) => {
                    self.push_routine(routine);
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    fn declaration_group_end(&self, items: &[SemItem], start: usize) -> usize {
        let SemItem::Declaration(first) = &items[start] else {
            return start + 1;
        };
        if !is_var_declaration(first) {
            return start + 1;
        }

        let mut end = start + 1;
        while end < items.len() {
            let SemItem::Declaration(next) = &items[end] else {
                break;
            };
            if !is_var_declaration(next) || next.group_span != first.group_span {
                break;
            }
            end += 1;
        }
        end
    }

    fn push_declaration_group(&mut self, scope: SemIrDeclarationScope, items: &'a [SemItem]) {
        let declarations = items
            .iter()
            .filter_map(|item| match item {
                SemItem::Declaration(decl) => Some(decl),
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some(first) = declarations.first().copied() else {
            return;
        };

        let index = self.declaration_groups.len();
        for declaration in &declarations {
            self.declarations_by_name
                .entry(declaration.symbol.name.clone())
                .or_insert(index);
        }

        self.declaration_groups.push(SemIrDeclarationGroup {
            scope,
            span: first.group_span,
            kind: declaration_group_kind(first),
            declarations,
        });
    }

    fn push_routine(&mut self, routine: &'a SemRoutine) {
        let local_group_indexes = self.collect_local_groups(routine);
        let routine_index = self.routines.len();
        self.routines_by_name
            .entry(routine.symbol.name.clone())
            .or_insert(routine_index);
        self.routines.push(SemIrRoutineView {
            routine,
            params: routine
                .params
                .iter()
                .map(|param| SemIrParamView {
                    param,
                    lvalue_shape: SemIrLValueShape::Symbol,
                })
                .collect(),
            local_group_indexes,
            body_summary: summarize_body(&routine.body),
        });
    }

    fn collect_local_groups(&mut self, routine: &'a SemRoutine) -> Vec<usize> {
        let mut indexes = Vec::new();
        let mut index = 0usize;
        while index < routine.locals.len() {
            let first = &routine.locals[index];
            let mut end = index + 1;
            if is_var_declaration(first) {
                while end < routine.locals.len()
                    && is_var_declaration(&routine.locals[end])
                    && routine.locals[end].group_span == first.group_span
                {
                    end += 1;
                }
            }

            let group_index = self.declaration_groups.len();
            for declaration in &routine.locals[index..end] {
                self.declarations_by_name
                    .entry(declaration.symbol.name.clone())
                    .or_insert(group_index);
            }
            self.declaration_groups.push(SemIrDeclarationGroup {
                scope: SemIrDeclarationScope::Routine(routine.symbol.name.clone()),
                span: first.group_span,
                kind: declaration_group_kind(first),
                declarations: routine.locals[index..end].iter().collect(),
            });
            indexes.push(group_index);
            index = end;
        }
        indexes
    }

    fn first_span(&self) -> Option<Span> {
        self.declaration_groups
            .first()
            .map(|group| group.span)
            .or_else(|| self.routines.first().map(|routine| routine.routine.span))
    }

    fn summary(&self) -> String {
        let local_groups = self
            .declaration_groups
            .iter()
            .filter(|group| matches!(group.scope, SemIrDeclarationScope::Routine(_)))
            .count();
        let global_groups = self.declaration_groups.len().saturating_sub(local_groups);
        let grouped_declarations = self
            .declaration_groups
            .iter()
            .map(|group| group.declarations.len().saturating_sub(1))
            .sum::<usize>();
        let type_groups = self
            .declaration_groups
            .iter()
            .filter(|group| matches!(group.kind, SemIrDeclarationGroupKind::Type))
            .count();
        let record_groups = self
            .declaration_groups
            .iter()
            .filter(|group| matches!(group.kind, SemIrDeclarationGroupKind::Record))
            .count();
        let params = self
            .routines
            .iter()
            .map(|routine| routine.params.len())
            .sum::<usize>();
        let array_params = self
            .routines
            .iter()
            .flat_map(|routine| &routine.params)
            .filter(|param| matches!(param.param.storage, SemParamStorage::Array))
            .count();
        let symbol_params = self
            .routines
            .iter()
            .flat_map(|routine| &routine.params)
            .filter(|param| matches!(param.lvalue_shape, SemIrLValueShape::Symbol))
            .count();
        let routine_local_groups = self
            .routines
            .iter()
            .map(|routine| routine.local_group_indexes.len())
            .sum::<usize>();
        let statements = self
            .routines
            .iter()
            .map(|routine| routine.body_summary.statements)
            .sum::<usize>();
        let machine_blocks = self
            .routines
            .iter()
            .map(|routine| routine.body_summary.machine_blocks)
            .sum::<usize>();
        let profile = match self.profile {
            CodegenProfile::Compat => "legacy",
            CodegenProfile::Modern => "modern",
        };
        format!(
            "read model: origin=${:04X}, profile={}, global_groups={}, local_groups={}, routine_local_groups={}, grouped_declarations={}, type_groups={}, record_groups={}, routines={}, params={}, array_params={}, symbol_params={}, statements={}, machine_blocks={}",
            self.origin,
            profile,
            global_groups,
            local_groups,
            routine_local_groups,
            grouped_declarations,
            type_groups,
            record_groups,
            self.routines.len(),
            params,
            array_params,
            symbol_params,
            statements,
            machine_blocks
        )
    }
}

fn declaration_group_kind(decl: &SemDeclaration) -> SemIrDeclarationGroupKind {
    match decl.storage {
        SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. } => {
            SemIrDeclarationGroupKind::Variables
        }
        SemDeclarationStorage::Type { .. } => SemIrDeclarationGroupKind::Type,
        SemDeclarationStorage::Record { .. } => SemIrDeclarationGroupKind::Record,
    }
}

fn is_var_declaration(decl: &SemDeclaration) -> bool {
    matches!(
        decl.storage,
        SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. }
    )
}

fn lvalue_shape(lvalue: &SemLValue) -> SemIrLValueShape {
    match &lvalue.kind {
        SemLValueKind::Symbol(_) => SemIrLValueShape::Symbol,
        SemLValueKind::UnresolvedName(_) => SemIrLValueShape::UnresolvedName,
        SemLValueKind::Deref { .. } => SemIrLValueShape::Deref,
        SemLValueKind::Index { syntax, .. } => match syntax {
            SemIndexSyntax::Call => SemIrLValueShape::CallIndex,
            SemIndexSyntax::Index => SemIrLValueShape::Index,
        },
        SemLValueKind::Field { .. } => SemIrLValueShape::Field,
    }
}

fn native_symbol_scope_key(scope: &CodegenSymbolScope) -> (&str, &str) {
    match scope {
        CodegenSymbolScope::Global => ("", ""),
        CodegenSymbolScope::Routine(name) => ("routine", name.as_str()),
    }
}

fn semir_native_routine_signature(routine: &SemIrRoutineView<'_>) -> CodegenRoutineSignature {
    let params = routine
        .params
        .iter()
        .map(|param| {
            let value_type = match param.param.storage {
                SemParamStorage::Value => param.param.ty.value.clone(),
                SemParamStorage::Array => ValueType::pointer_to(param.param.ty.value.clone()),
            };
            CodegenRoutineParam {
                name: param.param.symbol.name.clone(),
                width: value_type.value_width_bytes().unwrap_or(1),
                type_name: value_type_trace_name(&value_type),
            }
        })
        .collect();
    let (kind, return_type, return_width) = match routine.routine.signature.kind {
        RoutineKind::Proc => ("PROC".to_string(), None, None),
        RoutineKind::Func { return_type } => {
            let ty = ValueType::fund(return_type);
            (
                "FUNC".to_string(),
                Some(value_type_trace_name(&ty)),
                ty.value_width_bytes(),
            )
        }
    };
    CodegenRoutineSignature {
        name: routine.routine.symbol.name.clone(),
        kind,
        params,
        return_type,
        return_width,
    }
}

fn value_type_trace_name(value: &ValueType) -> String {
    let mut text = match &value.base {
        crate::semantic::ValueTypeBase::Fund(fund) => format!("{fund:?}").to_ascii_uppercase(),
        crate::semantic::ValueTypeBase::Named(name) => name.clone(),
        crate::semantic::ValueTypeBase::Callable(callable) => match &callable.kind {
            RoutineKind::Proc => "PROC".to_string(),
            RoutineKind::Func { return_type } => {
                format!("{}FUNC", format!("{return_type:?}").to_ascii_uppercase())
            }
        },
        crate::semantic::ValueTypeBase::Error => "ERROR".to_string(),
    };
    if value.pointer {
        text.push('*');
    }
    text
}

fn native_slot_size(slot: &NativeStorageSlot) -> u16 {
    slot.array
        .map(|array| match array.storage {
            CodegenArrayStorage::Inline => array.element_width.saturating_mul(array.len),
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => slot.width,
        })
        .unwrap_or(slot.width)
}

fn slot_is_array_pointer_value(slot: &NativeStorageSlot) -> bool {
    slot.width == 2
        && slot
            .array
            .is_some_and(|array| array.storage == CodegenArrayStorage::Pointer)
}

fn native_zero_page(address: u16) -> Option<ZeroPage> {
    (address < 0x100).then(|| ZeroPage::new(address as u8))
}

fn native_args_slot(width: u16) -> NativeResolvedSlot {
    NativeResolvedSlot {
        address: u16::from(runtime_zp::ARGS.address()),
        width,
        pointee_width: None,
        record: None,
    }
}

fn native_args_offset_slot(offset: u16, width: u16) -> Result<NativeResolvedSlot, String> {
    Ok(NativeResolvedSlot {
        address: u16::from(runtime_zp::ARGS.address())
            .checked_add(offset)
            .ok_or_else(|| "call argument offset address overflow".to_string())?,
        width,
        pointee_width: None,
        record: None,
    })
}

fn native_afcur_slot(width: u16) -> NativeResolvedSlot {
    NativeResolvedSlot {
        address: u16::from(runtime_zp::AFCUR.address()),
        width,
        pointee_width: None,
        record: None,
    }
}

fn native_sized_byte_array_storage_bytes(byte_size: u16, len: u16) -> Vec<u8> {
    let mut bytes = vec![0; usize::from(byte_size)];
    let len = Immediate::new(len);
    if bytes.len() > 2 {
        bytes[2] = len.low();
    }
    if bytes.len() > 3 {
        bytes[3] = len.high();
    }
    bytes
}

fn native_indexed_storage_for_array(storage: CodegenArrayStorage) -> NativeIndexedStorage {
    match storage {
        CodegenArrayStorage::Inline => NativeIndexedStorage::Inline,
        CodegenArrayStorage::Descriptor => NativeIndexedStorage::Descriptor,
        CodegenArrayStorage::Pointer => NativeIndexedStorage::ArrayPointer,
    }
}

fn native_address_kind_for_array(storage: CodegenArrayStorage) -> NativeAddressKind {
    match storage {
        CodegenArrayStorage::Inline => NativeAddressKind::StorageBase,
        CodegenArrayStorage::Descriptor | CodegenArrayStorage::Pointer => {
            NativeAddressKind::StoragePointer
        }
    }
}

fn native_call_debug_name(call: &SemCall) -> String {
    match &call.callee {
        SemCallable::User(symbol) => symbol.name.clone(),
        SemCallable::Builtin(symbol) => symbol.name.clone(),
        SemCallable::Indirect { .. } => "indirect".to_string(),
        SemCallable::Runtime { name, .. } => name.clone(),
    }
}

fn native_expr_debug_name(expr: &SemExpr) -> String {
    match &expr.kind {
        SemExprKind::Symbol(symbol) => symbol.name.clone(),
        SemExprKind::LValue(lvalue) => native_lvalue_debug_name(lvalue),
        SemExprKind::ArrayDecay(decay) => native_lvalue_debug_name(&decay.array),
        SemExprKind::ImplicitAddressOf(address) => native_lvalue_debug_name(&address.place),
        SemExprKind::Call(call) => match &call.callee {
            SemCallable::User(symbol) => format!("{}(...)", symbol.name),
            SemCallable::Builtin(symbol) => format!("{}(...)", symbol.name),
            SemCallable::Indirect { .. } => "indirect(...)".to_string(),
            SemCallable::Runtime { name, .. } => format!("{name}(...)"),
        },
        SemExprKind::Literal(_) => "literal".to_string(),
        SemExprKind::Binary { .. } => "binary expression".to_string(),
        SemExprKind::Unary { .. } => "unary expression".to_string(),
        SemExprKind::AddressOf(_) => "address-of expression".to_string(),
        SemExprKind::AddressOfSymbol(symbol) => format!("@{}", symbol.name),
        SemExprKind::Cast { expr, .. } => native_expr_debug_name(expr),
        SemExprKind::CurrentLocation => "*".to_string(),
        SemExprKind::UnresolvedName(name) | SemExprKind::Raw(name) => name.clone(),
        SemExprKind::Missing => "missing expression".to_string(),
    }
}

fn native_lvalue_debug_name(lvalue: &SemLValue) -> String {
    match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => symbol.name.clone(),
        SemLValueKind::UnresolvedName(name) => name.clone(),
        SemLValueKind::Deref { pointer } => format!("{}^", native_expr_debug_name(pointer)),
        SemLValueKind::Index { base, .. } => format!("{}(...)", native_expr_debug_name(base)),
        SemLValueKind::Field { base, field, .. } => {
            format!("{}.{}", native_lvalue_debug_name(base), field.name)
        }
    }
}

fn machine_block_name_offset(items: &[MachineItem]) -> (u16, usize) {
    let [
        MachineItem::Raw(op),
        MachineItem::Number(crate::lexer::NumberLiteral {
            value: Some(value), ..
        }),
        ..,
    ] = items
    else {
        return (0, 0);
    };
    match op.as_str() {
        "+" => (*value, 2),
        "-" => (0u16.wrapping_sub(*value), 2),
        _ => (0, 0),
    }
}

fn native_machine_opcode_operand_bytes(opcode: u8) -> u8 {
    decode_instruction(opcode)
        .map(|instruction| instruction.len.saturating_sub(1) as u8)
        .unwrap_or(0)
}

fn native_machine_number_with_offset(
    number: &crate::lexer::NumberLiteral,
    offset: i32,
    text: &str,
) -> Result<u16, String> {
    let value = number
        .value
        .ok_or_else(|| format!("machine block item `{text}` does not fit in 16 bits"))?;
    native_machine_apply_offset(value, offset, text)
}

fn native_machine_address_expr_uses_caret(expr: &MachineAddressExpr) -> bool {
    expr.text.contains('^')
}

fn native_machine_apply_offset(value: u16, offset: i32, text: &str) -> Result<u16, String> {
    let offset = u16::try_from(offset.rem_euclid(0x1_0000))
        .map_err(|_| format!("machine block item `{text}` does not fit in 16 bits"))?;
    Ok(value.wrapping_add(offset))
}

fn is_current_location_expr(expr: &SemExpr) -> bool {
    matches!(expr.kind, SemExprKind::CurrentLocation)
}

fn sem_logical_binary_condition(left: &SemExpr, right: &SemExpr) -> bool {
    sem_condition_shaped_expr(left) || sem_condition_shaped_expr(right)
}

fn sem_condition_shaped_expr(expr: &SemExpr) -> bool {
    match &expr.kind {
        SemExprKind::Cast { expr, .. } => sem_condition_shaped_expr(expr),
        SemExprKind::Binary { op, left, right } => {
            sem_is_compare_op(*op)
                || (matches!(op, BinaryOp::And | BinaryOp::Or)
                    && sem_logical_binary_condition(left, right))
        }
        _ => false,
    }
}

fn sem_is_compare_op(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    )
}

fn native_define_artifact_local(decl: &SemDeclaration) -> bool {
    matches!(decl.storage, SemDeclarationStorage::Scalar)
        && decl.initializer.is_none()
        && decl.ty.value.has_record_base()
}

fn routine_return_width(routine: &SemRoutine) -> Option<u16> {
    routine
        .callable_type
        .return_type
        .as_ref()
        .and_then(ValueType::value_width_bytes)
}

fn expr_width(expr: &SemExpr) -> Option<u16> {
    expr.type_facts().width
}

fn native_literal_width(value: u16) -> u16 {
    if value <= 0x00FF { 1 } else { 2 }
}

fn byte_lsh_result_is_zero(expr: &SemExpr) -> bool {
    if expr_width(expr) != Some(1) {
        return false;
    }
    let SemExprKind::Binary {
        op: BinaryOp::Lsh,
        right,
        ..
    } = &expr.kind
    else {
        return false;
    };
    literal_byte(right).is_some_and(|count| count >= 8)
}

fn native_byte_runtime_operand_is_supported(expr: &SemExpr) -> bool {
    expr_width(expr) == Some(1) || literal_byte(expr).is_some()
}

fn byte_runtime_helper(op: BinaryOp) -> Result<RuntimeHelperSlot, String> {
    match op {
        BinaryOp::Div => Ok(RuntimeHelperSlot::Div),
        BinaryOp::Mod => Ok(RuntimeHelperSlot::Mod),
        _ => Err("only byte DIV and MOD runtime helpers are supported".to_string()),
    }
}

fn native_builtin_system_address(name: &str) -> Option<u16> {
    Some(match normalize_name(name).as_str() {
        "GRAPHICS" => 0xA654,
        "PLOT" => 0xA6C3,
        "DRAWTO" => 0xA68C,
        "SCOMPARE" => 0xA864,
        "PRINTF" => 0xA3CC,
        "PRINT" => 0xA47F,
        "PRINTBE" => 0xA4EC,
        "PRINTD" => 0xA486,
        "INPUTMD" => 0xA499,
        "XIO" => 0xA4DE,
        "ZERO" => 0xA78A,
        "CLOSE" => 0xA479,
        "OPEN" => 0xA444,
        "ERROR" => 0x04CB,
        "BREAK" => 0xA7DA,
        "PUTD" => 0xA4D1,
        "PUT" => 0xA4CE,
        "PUTE" => 0xA4CC,
        "PUTDE" => 0xA4DA,
        _ => return None,
    })
}

fn native_builtin_variable_slot(name: &str) -> Option<NativeResolvedSlot> {
    let variable = resident_variable(name)?;
    if !matches!(variable.kind, ResidentVariableKind::Byte) {
        return None;
    }
    Some(NativeResolvedSlot {
        address: variable.address,
        width: 1,
        pointee_width: None,
        record: None,
    })
}

fn native_builtin_array_storage_slot(name: &str) -> Option<NativeStorageSlot> {
    let variable = resident_variable(name)?;
    let ResidentVariableKind::ByteArray { len } = variable.kind else {
        return None;
    };
    Some(NativeStorageSlot {
        address: variable.address,
        width: 1,
        array: Some(NativeArrayStorage {
            element_width: 1,
            len,
            storage: CodegenArrayStorage::Inline,
        }),
        pointee_width: None,
        record: None,
    })
}

fn literal_byte(expr: &SemExpr) -> Option<u8> {
    match &expr.kind {
        SemExprKind::Cast { expr, .. } => literal_byte(expr),
        SemExprKind::Literal(SemLiteral::Number(number)) => u8::try_from(number.value?).ok(),
        SemExprKind::Literal(SemLiteral::Char(ch)) => {
            let codepoint = u32::from(*ch);
            u8::try_from(codepoint).ok()
        }
        SemExprKind::Unary { op, expr } => {
            let value = literal_byte(expr)?;
            match op {
                UnaryOp::Plus => Some(value),
                UnaryOp::Neg => Some(0u8.wrapping_sub(value)),
                UnaryOp::AddressOf | UnaryOp::Deref => None,
            }
        }
        _ => None,
    }
}

fn literal_word(expr: &SemExpr) -> Option<u16> {
    match &expr.kind {
        SemExprKind::Cast { expr, .. } => literal_word(expr),
        SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
        SemExprKind::Literal(SemLiteral::Char(ch)) => {
            let codepoint = u32::from(*ch);
            u16::try_from(codepoint).ok()
        }
        _ => None,
    }
}

fn raw_initializer_bytes(expr: &SemExpr, element_width: u16) -> Result<Option<Vec<u8>>, String> {
    let Some(values) = raw_initializer_values(expr)? else {
        return Ok(None);
    };
    Ok(Some(numeric_storage_bytes(
        &values,
        element_width,
        Some(values.len() as u16),
    )))
}

fn raw_initializer_values(expr: &SemExpr) -> Result<Option<Vec<u16>>, String> {
    let SemExprKind::Raw(raw) = &expr.kind else {
        return Ok(None);
    };
    let text = raw.trim();
    let Some(inner) = text
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    else {
        return Ok(None);
    };
    let mut values = Vec::new();
    let mut sign = 1i32;
    for token in tokenize(inner).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("; ")
    })? {
        let value = match token.kind {
            TokenKind::Eof | TokenKind::Comma => continue,
            TokenKind::Plus => {
                sign = 1;
                continue;
            }
            TokenKind::Minus => {
                sign = -1;
                continue;
            }
            _ => parse_initializer_number_token(&token.kind)
                .ok_or_else(|| format!("unsupported raw initializer value {:?}", token.kind))?,
        };
        values.push(if sign < 0 {
            0u16.wrapping_sub(value)
        } else {
            value
        });
        sign = 1;
    }
    Ok(Some(values))
}

fn parse_initializer_number_token(token: &TokenKind) -> Option<u16> {
    match token {
        TokenKind::Number(number) => number.value,
        TokenKind::Char(ch) => source_char_byte(*ch).map(u16::from),
        TokenKind::Ident(name) => match normalize_name(name).as_str() {
            "TRUE" => Some(1),
            "FALSE" | "NIL" => Some(0),
            _ => None,
        },
        _ => None,
    }
}

fn parse_initializer_number_text(token: &str) -> Option<u16> {
    if let Some(hex) = token.strip_prefix('$') {
        return u16::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u16::from_str_radix(hex, 16).ok();
    }
    match normalize_name(token).as_str() {
        "TRUE" => return Some(1),
        "FALSE" | "NIL" => return Some(0),
        _ => {}
    }
    token.parse::<u16>().ok()
}

fn numeric_storage_bytes(values: &[u16], element_width: u16, explicit_len: Option<u16>) -> Vec<u8> {
    let len = explicit_len.unwrap_or(values.len() as u16);
    let mut bytes = Vec::with_capacity(usize::from(len.saturating_mul(element_width)));
    for index in 0..usize::from(len) {
        let value = values.get(index).copied().unwrap_or(0);
        bytes.push((value & 0x00FF) as u8);
        if element_width > 1 {
            bytes.push((value >> 8) as u8);
        }
    }
    bytes
}

fn string_initializer_bytes(
    expr: &SemExpr,
    explicit_len: Option<u16>,
) -> Result<Option<Vec<u8>>, String> {
    let SemExprKind::Literal(SemLiteral::String(text)) = &expr.kind else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    bytes.push(
        u8::try_from(text.chars().count()).map_err(|_| {
            "string initializer is too long for an ACTION! length prefix".to_string()
        })?,
    );
    for ch in text.chars() {
        bytes.push(
            source_char_byte(ch)
                .ok_or_else(|| format!("character `{ch}` is outside byte source encoding"))?,
        );
    }
    if let Some(len) = explicit_len
        && len > 0
    {
        bytes.resize(usize::from(len.saturating_add(1)), 0);
    }
    Ok(Some(bytes))
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

fn array_len_from_bytes(bytes: &[u8], element_width: u16) -> u16 {
    if element_width == 0 {
        return 0;
    }
    (bytes.len() / usize::from(element_width)) as u16
}

fn stmt_kind_name(stmt: &SemStmt) -> &'static str {
    match stmt {
        SemStmt::Define(_) => "define",
        SemStmt::Return { .. } => "return",
        SemStmt::Exit { .. } => "exit",
        SemStmt::Assign { .. } => "assignment",
        SemStmt::CompoundAssign { .. } => "compound assignment",
        SemStmt::Call { .. } => "call",
        SemStmt::MachineBlock { .. } => "machine block",
        SemStmt::If { .. } => "if",
        SemStmt::While { .. } => "while",
        SemStmt::DoUntil { .. } => "do-until",
        SemStmt::For { .. } => "for",
        SemStmt::Unsupported { .. } => "unsupported",
    }
}

fn stmt_span(stmt: &SemStmt) -> Span {
    match stmt {
        SemStmt::Define(define) => define.span,
        SemStmt::Return { span, .. }
        | SemStmt::Exit { span }
        | SemStmt::Assign { span, .. }
        | SemStmt::CompoundAssign { span, .. }
        | SemStmt::Call { span, .. }
        | SemStmt::MachineBlock { span, .. }
        | SemStmt::If { span, .. }
        | SemStmt::While { span, .. }
        | SemStmt::DoUntil { span, .. }
        | SemStmt::For { span, .. }
        | SemStmt::Unsupported { span, .. } => *span,
    }
}

fn summarize_body(stmts: &[SemStmt]) -> SemIrBodySummary {
    let mut summary = SemIrBodySummary::default();
    summarize_stmt_list(stmts, &mut summary);
    summary
}

fn summarize_stmt_list(stmts: &[SemStmt], summary: &mut SemIrBodySummary) {
    for stmt in stmts {
        summary.statements += 1;
        match stmt {
            SemStmt::Assign { target, .. } | SemStmt::CompoundAssign { target, .. } => {
                summary.assignments += 1;
                let _ = lvalue_shape(target);
            }
            SemStmt::Call { .. } => summary.calls += 1,
            SemStmt::Return { .. } => summary.returns += 1,
            SemStmt::MachineBlock { .. } => summary.machine_blocks += 1,
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                summary.branches += 1;
                for branch in branches {
                    summarize_stmt_list(&branch.body, summary);
                }
                summarize_stmt_list(else_body, summary);
            }
            SemStmt::While { body, .. } | SemStmt::DoUntil { body, .. } => {
                summary.loops += 1;
                summarize_stmt_list(body, summary);
            }
            SemStmt::For { target, body, .. } => {
                summary.loops += 1;
                let _ = lvalue_shape(target);
                summarize_stmt_list(body, summary);
            }
            SemStmt::Define(_) | SemStmt::Exit { .. } | SemStmt::Unsupported { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::semantic::{analyze, ir};

    use super::*;

    #[test]
    fn native_read_model_preserves_declaration_groups_and_routines() {
        let source = "BYTE a,b CARD c PROC Main(BYTE x) BYTE y,z y=x RETURN";
        let semir = lower_source(source);
        let model = SemIrReadModel::new(&semir, 0x3000, CodegenProfile::Compat);

        assert_eq!(model.routines.len(), 1);
        assert_eq!(model.routines[0].routine.symbol.name, "Main");
        assert_eq!(model.routines[0].params.len(), 1);
        assert_eq!(model.routines[0].local_group_indexes.len(), 1);
        assert_eq!(model.declaration_groups.len(), 3);
        assert_eq!(model.declaration_groups[0].declarations.len(), 2);
        assert_eq!(model.declaration_groups[1].declarations.len(), 1);
        assert_eq!(model.declaration_groups[2].declarations.len(), 2);
    }

    #[test]
    fn native_read_model_classifies_lvalue_shapes() {
        let source = "TYPE Pair=[BYTE tag] Pair rec BYTE ARRAY a(2) BYTE POINTER p BYTE b PROC Main() b=a(0) p^=b rec.tag=b RETURN";
        let semir = lower_source(source);
        let routine = semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("routine");

        let SemStmt::Assign { target, .. } = &routine.body[1] else {
            panic!("expected pointer assignment");
        };
        assert_eq!(lvalue_shape(target), SemIrLValueShape::Deref);

        let SemStmt::Assign { target, .. } = &routine.body[2] else {
            panic!("expected field assignment");
        };
        assert_eq!(lvalue_shape(target), SemIrLValueShape::Field);
    }

    #[test]
    fn native_output_map_records_storage_routines_and_source_ranges() {
        let source = "BYTE total BYTE FUNC Echo(BYTE n) RETURN(n) PROC Main() total=Echo(1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Compat).unwrap();

        assert!(output.map.storage_symbols.iter().any(|symbol| {
            symbol.name == "total"
                && symbol.scope == CodegenSymbolScope::Global
                && symbol.kind == CodegenSymbolKind::Storage
                && symbol.address == 0x3000
                && symbol.size == 1
        }));
        assert!(output.map.storage_symbols.iter().any(|symbol| {
            symbol.name == "n"
                && symbol.scope == CodegenSymbolScope::Routine("Echo".to_string())
                && symbol.kind == CodegenSymbolKind::Parameter
                && symbol.size == 1
        }));
        assert!(
            output
                .map
                .routine_ranges
                .iter()
                .any(|range| range.name == "Echo" && range.end > range.start)
        );
        assert!(output.map.source_ranges.iter().any(|range| {
            range.kind == CodegenSourceRangeKind::Declaration
                && range.name.as_deref() == Some("total")
        }));
        assert!(output.map.source_ranges.iter().any(|range| {
            range.kind == CodegenSourceRangeKind::Routine
                && range.name.as_deref() == Some("Main")
                && range.end > range.start
        }));
        assert!(output.map.source_ranges.iter().any(|range| {
            range.kind == CodegenSourceRangeKind::Statement
                && range.name.as_deref() == Some("assignment")
                && range.end > range.start
        }));
    }

    #[test]
    fn native_run_address_falls_back_to_last_routine_without_main() {
        let source = "PROC First() RETURN PROC NavInit() First() RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(output.run_address, routine_address(&output, "NavInit"));
        assert_eq!(output.map.run_address, output.run_address);
    }

    #[test]
    fn native_run_address_prefers_main_over_later_routines() {
        let source = "PROC Main() RETURN PROC Later() RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(output.run_address, routine_address(&output, "Main"));
        assert_eq!(output.map.run_address, output.run_address);
    }

    #[test]
    fn native_scalar_initializers_resolve_prior_storage_symbols() {
        let source = "BYTE zx=$5A,zy=zx+1 BYTE POINTER screen BYTE scl=screen,sch=screen+1 BYTE ARRAY fname(15) BYTE fnamelen=fname PROC Main() zy=1 sch=2 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        for (name, address) in [
            ("zx", 0x005A),
            ("zy", 0x005B),
            ("screen", 0x3000),
            ("scl", 0x3000),
            ("sch", 0x3001),
            ("fname", 0x3002),
            ("fnamelen", 0x3002),
        ] {
            let symbol = output
                .map
                .storage_symbols
                .iter()
                .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
                .unwrap_or_else(|| panic!("missing storage symbol {name}"));
            assert_eq!(symbol.address, address, "address for {name}");
        }
    }

    #[test]
    fn native_set_data_cursor_can_allocate_zero_page_storage() {
        let source = "SET $E=$E6 SET $F=$00 SET $491=$E6 SET $492=$00 BYTE POINTER screen BYTE scl=screen,sch=screen+1 SET $E=$3000 SET $491=$3000 PROC Main=*() screen=$4000 [$A5 scl $E6 sch $60]";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        for (name, address) in [("screen", 0x00E6), ("scl", 0x00E6), ("sch", 0x00E7)] {
            let symbol = output
                .map
                .storage_symbols
                .iter()
                .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
                .unwrap_or_else(|| panic!("missing storage symbol {name}"));
            assert_eq!(symbol.address, address, "address for {name}");
        }
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::STA_ZP, 0xE7])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::STA_ZP, 0xE6])
        );
        assert!(
            output
                .bytes
                .windows(5)
                .any(|bytes| bytes == [opcode::LDA_ZP, 0xE6, opcode::INC_ZP, 0xE7, opcode::RTS])
        );
    }

    #[test]
    fn native_set_symbol_to_current_location_patches_unsized_array_pointer() {
        let source = "BYTE ARRAY buffer PROC Main() RETURN SET buffer=*";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        let patched = u16::from_le_bytes([output.bytes[0], output.bytes[1]]);
        assert_eq!(
            patched,
            output
                .origin
                .wrapping_add(output.bytes.len() as u16)
                .wrapping_sub(1)
        );
        let symbol = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "buffer")
            .expect("buffer symbol");
        assert_eq!(symbol.array, Some(CodegenArrayStorage::Pointer));
        assert_eq!(symbol.size, 2);
    }

    #[test]
    fn native_set_symbol_to_current_location_uses_deferred_storage_high_water() {
        let source = "BYTE ARRAY buffer PROC UsesBacking() BYTE ARRAY temp(300) temp(0)=1 RETURN PROC Main() UsesBacking() RETURN SET buffer=*";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        let buffer = output
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name == "buffer")
            .expect("buffer symbol");
        let offset = usize::from(buffer.address.wrapping_sub(output.origin));
        let patched = u16::from_le_bytes([output.bytes[offset], output.bytes[offset + 1]]);
        let skipped_end = output
            .skipped_ranges
            .iter()
            .map(|range| range.start.wrapping_add(range.len))
            .max()
            .unwrap();

        assert_eq!(patched, skipped_end);
    }

    #[test]
    fn native_raw_initializers_accept_truth_symbols() {
        let source = "DEFINE true=\"1\", false=\"0\", nil=\"0\" BYTE first=[true], second=[false], third=[nil] PROC Main() RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(&output.bytes[..3], &[1, 0, 0]);
    }

    #[test]
    fn native_call_args_accept_numeric_defines() {
        let source = "DEFINE CLEAR=\"$00\" BYTE seen PROC Take(BYTE mode,dx,dy) seen=mode RETURN PROC Main() Take(CLEAR,1,2) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_IMM, 0])
        );
    }

    #[test]
    fn native_call_args_accept_negative_byte_literals() {
        let source = "BYTE seen PROC Take(BYTE a,b) seen=b RETURN PROC Main() Take(0,-1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDX_IMM, 0xFF])
        );
    }

    #[test]
    fn native_byte_assignments_accept_negative_literals() {
        let source = "BYTE j PROC Main() j=-1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_IMM, 0xFF])
        );
    }

    #[test]
    fn native_returns_accept_numeric_defines() {
        let source = "DEFINE nil=\"0\" CARD FUNC Missing() RETURN(nil)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::RTS));
        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x84, 0xA0]));
        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x84, 0xA1]));
    }

    #[test]
    fn native_call_args_accept_byte_expressions_in_x_and_y_slots() {
        let source = "BYTE max,sink PROC Take(BYTE a,b,c) sink=a RETURN PROC Main() max=5 Take(1,max-1,max+1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ADC_IMM));
        assert!(output.bytes.contains(&opcode::SBC_IMM));
        assert!(output.bytes.contains(&opcode::TAX));
        assert!(output.bytes.contains(&opcode::TAY));
    }

    #[test]
    fn native_sargs_call_args_share_inline_string_addresses() {
        let source = "CHAR out PROC Take(CHAR POINTER s BYTE n CHAR POINTER p) out=s^ RETURN PROC Main() Take(\"AB\",1,@out) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JMP_ABS));
        assert!(output.bytes.contains(&opcode::LDX_IMM));
        assert!(output.bytes.contains(&opcode::LDA_IMM));
        assert!(output.bytes.contains(&opcode::JSR_ABS));
    }

    #[test]
    fn native_builtin_calls_use_shared_argument_materialization() {
        let source = "BYTE b BYTE POINTER p PROC Main() Print(\"X\") PrintF(\"$H\",p) PrintBE(b) Put(b) PutE() RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        for address in [0xA47F, 0xA3CC, 0xA4EC, 0xA4CE, 0xA4CC] {
            assert!(output.bytes.windows(3).any(|bytes| {
                bytes
                    == [
                        opcode::JSR_ABS,
                        (address & 0x00FF) as u8,
                        (address >> 8) as u8,
                    ]
            }));
        }
    }

    #[test]
    fn native_user_system_address_calls_emit_jsr_absolute() {
        let source = "PROC CIO=$E456(BYTE areg,xreg) PROC Main() CIO(1,2) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| { bytes == [opcode::JSR_ABS, 0x56, 0xE4] })
        );
        assert!(
            output
                .routine_addresses
                .iter()
                .any(|routine| routine.name == "CIO" && routine.address == 0xE456)
        );
    }

    #[test]
    fn native_predefined_variables_use_builtin_storage() {
        let source = "BYTE d PROC Main() color=3 LIST=1 TRACE=0 d=EOF(1) d=device RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes == [opcode::STA_ABS, 0xFD, 0x02])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_ZP, 0xB7])
        );
        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes == [opcode::STA_ABS, 0x9A, 0x04])
        );
        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes == [opcode::STA_ABS, 0xC3, 0x04])
        );
        assert!(
            output
                .bytes
                .windows(3)
                .any(|bytes| bytes == [opcode::LDA_ABS, 0xC1, 0x05])
        );
    }

    #[test]
    fn native_local_record_type_declarations_register_layout() {
        let source = "PROC Main() TYPE IOCB=[BYTE cmd CARD addr] IOCB POINTER p p=$340 p.cmd=1 p.addr=$4000 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_statement_defines_are_declarations() {
        let source = "PROC Main() DEFINE One=\"1\" RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_define_calls_emit_machine_bodies() {
        let source = "PROC Main() DEFINE Nop=\"[$EA]\" Nop RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&0xEA));
        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_sargs_call_args_stage_computed_word_arguments() {
        let source = "PROC MovePage(CARD dst,src BYTE len) RETURN PROC Main(CHAR POINTER s,t) MovePage(s+s^+1,t+1,t^) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::STA_ZP, runtime_zp::ARGS.offset(3).address(),])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDY_ZP, runtime_zp::ARGS.offset(2).address(),])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address(),])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address(),])
        );
    }

    #[test]
    fn native_word_call_args_zero_extend_byte_expressions() {
        let source = "PROC Take(CARD n BYTE tag) RETURN PROC Main(BYTE n) Take(n+1,7) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ADC_IMM));
        assert!(output.bytes.windows(4).any(|bytes| bytes
            == [
                opcode::LDA_IMM,
                0,
                opcode::STA_ZP,
                runtime_zp::ARGS.offset(1).address(),
            ]));
    }

    #[test]
    fn native_sargs_call_args_accept_byte_sized_card_expressions() {
        let source = "CARD ARRAY v(4) BYTE i,a PROC Store(CARD p BYTE value) RETURN PROC Main() Store(v(i),a!$7F) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::TAY));
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address(),])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address(),])
        );
    }

    #[test]
    fn native_sargs_runtime_set_uses_generated_helper() {
        let source = "SET $4EE=r_Par PROC r_Par() RETURN PROC Take(BYTE a,b,c) RETURN PROC Main() Take(1,2,3) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();
        let r_par = output
            .routine_addresses
            .iter()
            .find(|routine| routine.name == "r_Par")
            .expect("r_Par address");

        assert!(!output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_SARGS.low(),
                    runtime_helper::CARTRIDGE_SARGS.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    (r_par.address & 0x00FF) as u8,
                    (r_par.address >> 8) as u8,
                ]
        }));
    }

    #[test]
    fn native_routine_entry_forgets_known_y_for_array_parameter_reads() {
        let source = "CARD POINTER p CARD out BYTE sink PROC Seed() out=p^ RETURN PROC Print(BYTE ARRAY s) BYTE i FOR i=1 TO s(0) DO sink=s(i) OD RETURN PROC Main(BYTE ARRAY s) Seed() Print(s) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();
        let print = usize::from(routine_address(&output, "Print") - output.origin);
        let main = usize::from(routine_address(&output, "Main") - output.origin);
        let print_bytes = &output.bytes[print..main];

        assert!(print_bytes.windows(4).any(|bytes| {
            bytes
                == [
                    opcode::LDY_IMM,
                    0,
                    opcode::CMP_IZY,
                    runtime_zp::ARRAY_ADDR.address(),
                ]
        }));
        assert!(
            print_bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_IZY, runtime_zp::ARRAY_ADDR.address()])
        );
    }

    #[test]
    fn native_calls_forget_known_y_before_indexed_reads() {
        let source = "BYTE POINTER p BYTE out PROC Touch=*(BYTE ch) [$A8 $60] PROC Main() out=p^ Touch(out) out=p(2) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();
        let touch = routine_address(&output, "Touch");
        let main = usize::from(routine_address(&output, "Main") - output.origin);
        let main_bytes = &output.bytes[main..];
        let call = [opcode::JSR_ABS, (touch & 0x00FF) as u8, (touch >> 8) as u8];
        let call_at = main_bytes
            .windows(call.len())
            .position(|bytes| bytes == call)
            .expect("expected Main to call Touch");
        let after_call = &main_bytes[call_at + call.len()..];

        assert!(after_call.windows(4).any(|bytes| {
            bytes
                == [
                    opcode::LDY_IMM,
                    0,
                    opcode::LDA_IZY,
                    runtime_zp::ARRAY_ADDR.address(),
                ]
        }));
    }

    #[test]
    fn native_labels_forget_known_y_before_card_array_reads() {
        let source = "CARD ARRAY v(4) CARD s BYTE files,j PROC Main() s=v(0) files=0 FOR j=1 TO files DO s=1 OD s=v(files) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();
        let main = usize::from(routine_address(&output, "Main") - output.origin);
        let main_bytes = &output.bytes[main..];

        assert!(main_bytes.windows(4).any(|bytes| {
            bytes
                == [
                    opcode::LDY_IMM,
                    1,
                    opcode::LDA_IZY,
                    runtime_zp::ARRAY_ADDR.address(),
                ]
        }));
    }

    #[test]
    fn native_machine_blocks_resolve_storage_offsets_and_address_bytes() {
        let source = "CARD r=$86 BYTE x PROC Main=*() [$A5 r+1 <x >x $60]";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(&output.bytes[..6], &[0x00, 0xA5, 0x87, 0x00, 0x30, 0x60]);
    }

    #[test]
    fn native_machine_block_caret_symbol_emits_compile_time_address() {
        let source =
            "BYTE ARRAY screen=$8010,text=$9E80 PROC DL15=*() [78 screen^ 66 text^ 65 DL15]";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();
        let routine = output
            .routine_addresses
            .iter()
            .find(|routine| routine.name == "DL15")
            .expect("expected DL15 routine address");

        assert!(
            output
                .bytes
                .windows(7)
                .any(|bytes| bytes == [0x4E, 0x10, 0x80, 0x42, 0x80, 0x9E, 0x41])
        );
        assert!(
            output.bytes.windows(3).any(|bytes| {
                bytes
                    == [
                        0x41,
                        (routine.address & 0x00FF) as u8,
                        (routine.address >> 8) as u8,
                    ]
            }),
            "expected bare routine label in machine block to emit a word"
        );
    }

    #[test]
    fn native_machine_block_caret_symbol_accepts_named_offset() {
        let source =
            "DEFINE OFF=\"2\" BYTE ARRAY screen=$8010 PROC Main() [screen^+OFF >screen^-OFF $60]";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(4)
                .any(|bytes| bytes == [0x12, 0x80, 0x80, 0x60])
        );
    }

    #[test]
    fn native_machine_blocks_resolve_routine_labels() {
        let source = "PROC Helper=*() [$60] PROC Main=*() [$20 Helper <Helper >Helper $60]";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(
            &output.bytes[..7],
            &[0x60, 0x20, 0x00, 0x30, 0x00, 0x30, 0x60]
        );
    }

    #[test]
    fn native_word_compound_assignments_support_add_sub_and_rsh() {
        let source = "CARD p=$86 PROC Main=*() p==+1 p==-2 p==rsh 1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(12).any(|bytes| {
            bytes
                == [
                    0x18, 0xA5, 0x86, 0x69, 0x01, 0x85, 0x86, 0xA5, 0x87, 0x69, 0x00, 0x85,
                ]
        }));
        assert!(output.bytes.windows(12).any(|bytes| {
            bytes
                == [
                    0x38, 0xA5, 0x86, 0xE9, 0x02, 0x85, 0x86, 0xA5, 0x87, 0xE9, 0x00, 0x85,
                ]
        }));
        assert!(
            output
                .bytes
                .windows(6)
                .any(|bytes| bytes == [0x4E, 0x87, 0x00, 0x6E, 0x86, 0x00])
        );
    }

    #[test]
    fn native_word_compound_assignments_accept_byte_expression_operands() {
        let source = "BYTE cell BYTE POINTER p CARD total PROC Main() p=@cell total==+p^+3 total==-p^+1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::ADC_ZP));
        assert!(output.bytes.contains(&opcode::SBC_ZP));
    }

    #[test]
    fn native_byte_array_assignments_accept_word_high_byte_shifts() {
        let source = "BYTE ARRAY quickmul(48) BYTE i CARD h PROC Main() i=0 h=$1234 quickmul(i+24)=h RSH 8 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::STA_ABS_X));
    }

    #[test]
    fn native_byte_subtraction_accepts_literal_left_operand() {
        let source = "BYTE x,y PROC Main() x=3 y=40-x RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::SBC_ABS));
    }

    #[test]
    fn native_byte_assignments_accept_constant_right_shift_expressions() {
        let source = "BYTE files,gap PROC Main() gap=files RSH 1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LSR_A));
    }

    #[test]
    fn native_byte_assignments_accept_byte_product_offsets() {
        let source = "BYTE winnum,zx PROC Main() zx=20*winnum+1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MUL.low(),
                    runtime_helper::CARTRIDGE_MUL.high(),
                ]
        }));
        assert!(output.bytes.contains(&opcode::ADC_IMM));
    }

    #[test]
    fn native_byte_expressions_accept_div_mod_helpers() {
        let source = "BYTE b,i PROC Putchar(BYTE ch) RETURN PROC Main() i=b/100 b==MOD 100 Putchar('0%(b MOD 10)) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_DIV.low(),
                    runtime_helper::CARTRIDGE_DIV.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MOD.low(),
                    runtime_helper::CARTRIDGE_MOD.high(),
                ]
        }));
        assert!(output.bytes.contains(&opcode::ORA_ZP));
    }

    #[test]
    fn native_byte_compounds_accept_logic_ops() {
        let source = "BYTE scrnum,mask PROC Main() scrnum==!1 scrnum==&mask RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::AND_ABS));
    }

    #[test]
    fn native_word_binary_return_expressions_use_return_slot() {
        let source = "CARD left,right CARD FUNC Diff() RETURN(left-right)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x85, 0xA0]));
        assert!(output.bytes.contains(&0xE5) || output.bytes.contains(&0xED));
        assert!(output.bytes.contains(&0x60));
    }

    #[test]
    fn native_word_binary_return_expressions_zero_extend_byte_left_operands() {
        let source = "INT FUNC Neg(INT x) RETURN(0-x)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x85, 0xA0]));
        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x85, 0xA1]));
        assert!(output.bytes.contains(&opcode::SBC_ABS));
        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_word_logic_expressions_accept_direct_word_operands() {
        let source = "CARD acc CARD FUNC Mask(CARD r) acc=acc XOR $0101 RETURN(r AND acc)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::AND_ABS));
        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_word_logic_expressions_zero_extend_byte_operands() {
        let source =
            "BYTE left,right,out PROC Main() IF ((left AND 1) XOR right) # 0 THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::AND_IMM));
        assert!(output.bytes.contains(&opcode::EOR_ABS));
        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_word_shift_expressions_use_runtime_helpers() {
        let source = "CARD r,out BYTE shift PROC Main() out=r LSH shift out=out RSH shift RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_LSH.low(),
                    runtime_helper::CARTRIDGE_LSH.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_RSH.low(),
                    runtime_helper::CARTRIDGE_RSH.high(),
                ]
        }));
        assert!(output.bytes.contains(&opcode::STX_ABS) || output.bytes.contains(&opcode::STX_ZP));
    }

    #[test]
    fn native_word_runtime_expressions_use_runtime_helpers() {
        let source = "CARD a,b,r PROC Main() r=a*b r=r/3 r=r MOD 5 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MUL.low(),
                    runtime_helper::CARTRIDGE_MUL.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_DIV.low(),
                    runtime_helper::CARTRIDGE_DIV.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MOD.low(),
                    runtime_helper::CARTRIDGE_MOD.high(),
                ]
        }));
    }

    #[test]
    fn native_word_runtime_compounds_use_runtime_helpers() {
        let source = "CARD r PROC Main() r==*2 r==/3 r==MOD 5 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MUL.low(),
                    runtime_helper::CARTRIDGE_MUL.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_DIV.low(),
                    runtime_helper::CARTRIDGE_DIV.high(),
                ]
        }));
        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MOD.low(),
                    runtime_helper::CARTRIDGE_MOD.high(),
                ]
        }));
    }

    #[test]
    fn native_byte_assignments_accept_word_runtime_low_byte() {
        let source = "BYTE d CARD n,base PROC Main() d=n MOD base RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MOD.low(),
                    runtime_helper::CARTRIDGE_MOD.high(),
                ]
        }));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_byte_assignments_accept_word_pointer_deref_low_byte() {
        let source = "BYTE b CARD POINTER p PROC Main() p=$4000 b=p^ RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_word_assignments_zero_extend_byte_values() {
        let source = "BYTE b INT w PROC Main() w=b RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_IMM, 0])
        );
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_byte_assignments_accept_word_array_low_byte() {
        let source = "BYTE b,i CARD ARRAY table(3) PROC Main() b=table(i) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_byte_arithmetic_accepts_word_low_byte_operands() {
        let source = "BYTE b INT w PROC Main() b=b+w RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ADC_ABS));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_byte_lsh_accepts_computed_operands() {
        let source = "BYTE ARRAY table(4) BYTE width,ntemp,temp PROC Main() temp=table(width) LSH (ntemp LSH 1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_LSH.low(),
                    runtime_helper::CARTRIDGE_LSH.high(),
                ]
        }));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_byte_rsh_accepts_computed_counts() {
        let source =
            "BYTE ARRAY table(8) BYTE i,count,out PROC Main() out=table(i) RSH count RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_RSH.low(),
                    runtime_helper::CARTRIDGE_RSH.high(),
                ]
        }));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_pointer_byte_index_compounds_preserve_address() {
        let source = "BYTE POINTER p BYTE mask CARD i PROC Main() p=$4000 i=3 p(i)==&mask RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::AND_ABS));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_word_array_indexes_accept_word_expressions() {
        let source = "CARD ARRAY table(91) CARD theta,out PROC Main() out=table(90-theta) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::RTS));
    }

    #[test]
    fn native_word_binary_expressions_accept_byte_offsets() {
        let source =
            "BYTE cell BYTE POINTER p CARD FUNC Next(BYTE POINTER menu) RETURN(menu+menu^+5)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::ADC_ZP));
        assert!(output.bytes.contains(&opcode::ADC_IMM));
    }

    #[test]
    fn native_word_binary_expressions_accept_array_decay_operands() {
        let source =
            "BYTE ARRAY buffer CARD memtop,len PROC Main() len=memtop-buffer RETURN SET buffer=*";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .iter()
                .filter(|byte| **byte == opcode::SBC_ABS)
                .count()
                >= 2
        );
    }

    #[test]
    fn native_word_binary_expressions_accept_record_field_operands() {
        let source = "TYPE Node=[CARD size] Node rec CARD total PROC Add(Node POINTER n) total=total+n.size RETURN PROC Main() Add(rec) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| { bytes == [opcode::STA_ZP, runtime_zp::AFCUR.offset(1).address()] })
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| { bytes == [opcode::ADC_ZP, runtime_zp::AFCUR.address()] })
        );
    }

    #[test]
    fn native_pointer_index_word_stores_accept_computed_word_values() {
        let source = "TYPE Item=[CARD value] CARD ARRAY words=[1 2 3 4] PROC Scatter(Item POINTER item CARD POINTER cp BYTE i) cp(i)=item.value+words(i) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::LDA_ABS_X));
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| { bytes == [opcode::ADC_ZP, runtime_zp::AFCUR.address()] })
        );
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_word_deref_return_expressions_use_return_slot() {
        let source = "CARD POINTER p CARD FUNC Read() RETURN(p^)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x85, 0xA0]));
        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x85, 0xA1]));
        assert!(output.bytes.contains(&0xB1));
    }

    #[test]
    fn native_deref_assignment_accepts_function_call_results() {
        let source = "BYTE POINTER p BYTE FUNC Internal(BYTE ch) RETURN(ch) PROC Main(BYTE ch) p^=Internal(ch) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.windows(2).any(|bytes| bytes == [0xA5, 0xA0]));
        assert!(output.bytes.contains(&0x91));
    }

    #[test]
    fn native_word_deref_assignment_preserves_address_for_call_results() {
        let source = "CARD POINTER p CARD FUNC Internal(CARD ch) RETURN(ch) PROC Main(CARD ch) p^=Internal(ch) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_word_deref_assignment_accepts_dynamic_word_array_reads() {
        let source = "CARD POINTER currentDir CARD ARRAY dirsectors(5) BYTE nestLevel PROC Main() currentDir^=dirsectors(nestLevel) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_byte_deref_compound_assignments_support_add_and_sub() {
        let source = "BYTE cell BYTE POINTER p PROC Main() p=@cell p^==+1 p^==-1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::ADC_IMM));
        assert!(output.bytes.contains(&opcode::SBC_IMM));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_byte_deref_compound_assignments_accept_deref_operands() {
        let source = "BYTE POINTER s,t PROC Main() s=$4000 t=$4100 s^==+t^ RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::ADC_ZP, runtime_zp::AFCUR.address(),])
        );
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_byte_deref_assignment_accepts_computed_deref_values() {
        let source = "BYTE cell BYTE POINTER p PROC Main() p=@cell p^=p^+1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::ADC_IMM));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_record_field_assignment_accepts_word_function_call_results() {
        let source = "TYPE Pair=[CARD value] Pair rec CARD FUNC Internal(CARD ch) RETURN(ch) PROC Main(CARD ch) rec.value=Internal(ch) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&0x91));
    }

    #[test]
    fn native_record_field_assignment_accepts_computed_byte_values() {
        let source = "TYPE Pair=[BYTE value] Pair rec PROC Touch(Pair POINTER p BYTE seed) p.value=seed+1 RETURN PROC Main() Touch(rec,3) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::ADC_IMM, 0x01])
        );
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_record_field_assignment_accepts_byte_field_values() {
        let source = "TYPE Pair=[BYTE left, right] Pair rec, other PROC Copy(Pair POINTER dst, src) dst.right=src.left RETURN PROC Main() Copy(rec,other) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_record_field_assignment_accepts_computed_field_byte_values() {
        let source = "TYPE Pair=[BYTE left, right, out] Pair rec, other PROC Copy(Pair POINTER dst, src) dst.out=src.left+src.right RETURN PROC Main() Copy(rec,other) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::ADC_ZP, runtime_zp::AFCUR.address()])
        );
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_direct_record_field_reads_are_addressable() {
        let source = "TYPE Item=[CARD value] Item first CARD w PROC Main() w=first.value RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_ABS));
        assert!(output.bytes.contains(&opcode::STA_ABS));
        assert!(!output.bytes.contains(&opcode::LDA_IZY));
    }

    #[test]
    fn native_for_loop_supports_byte_default_step() {
        let source = "BYTE i,sum PROC Main() FOR i=1 TO 3 DO sum==+i OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::CMP_IMM));
        assert!(output.bytes.contains(&opcode::BCC_REL));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
        assert!(output.bytes.contains(&opcode::INC_ABS));
    }

    #[test]
    fn native_for_loop_supports_card_default_step() {
        let source = "CARD i,total PROC Main() FOR i=1 TO 3 DO total==+i OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::CMP_IMM));
        assert!(output.bytes.contains(&opcode::BCC_REL));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
        assert!(output.bytes.windows(2).any(|bytes| bytes == [0x69, 0x01]));
    }

    #[test]
    fn native_for_loop_zero_extends_byte_bounds_for_card_targets() {
        let source = "CARD i BYTE max PROC Main() max=3 FOR i=1 TO max DO OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::CMP_IMM, 0x00])
        );
        assert!(output.bytes.contains(&opcode::CMP_ABS));
    }

    #[test]
    fn native_for_loop_accepts_pointer_backed_byte_end_bound() {
        let source = "DEFINE STRING=\"CHAR ARRAY\" STRING text(0)=\"ABC\" BYTE i,total PROC Touch(STRING s) FOR i=1 TO s(0) DO total==+1 OD RETURN PROC Main() Touch(text) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::CMP_IZY));
    }

    #[test]
    fn native_for_loop_accepts_computed_byte_end_bound() {
        let source =
            "BYTE i,files,total PROC Main() files=4 FOR i=1 TO files-1 DO total==+i OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::CMP_ZP, runtime_zp::AFCUR.address(),])
        );
    }

    #[test]
    fn native_for_loop_supports_explicit_positive_step() {
        let source = "BYTE i,sum PROC Main() FOR i=1 TO 5 STEP 2 DO sum==+i OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::ADC_IMM, 0x02])
        );
    }

    #[test]
    fn native_for_loop_supports_negative_step() {
        let source = "BYTE i,sum PROC Main() FOR i=5 TO 1 STEP -1 DO sum==+i OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::BCS_REL));
        assert!(output.bytes.contains(&opcode::DEC_ABS));
    }

    #[test]
    fn native_loop_exit_branches_to_loop_end() {
        let source = "BYTE i PROC Main() FOR i=1 TO 3 DO EXIT OD WHILE i<4 DO EXIT OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        let jmp_count = output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::JMP_ABS)
            .count();
        assert!(jmp_count >= 2);
    }

    #[test]
    fn native_do_until_supports_nonzero_conditions() {
        let source = "BYTE done,count PROC Main() DO count==+1 done=1 UNTIL done OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::BNE_REL));
    }

    #[test]
    fn native_long_forward_condition_branches_use_absolute_veneers() {
        let mut source = "BYTE flag,total PROC Main() IF flag THEN ".to_string();
        for _ in 0..90 {
            source.push_str("total==+1 ");
        }
        source.push_str("FI RETURN");

        let semir = lower_source(&source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::BNE_REL));
        assert!(output.bytes.contains(&opcode::JMP_ABS));
    }

    #[test]
    fn native_inline_byte_array_reads_accept_dynamic_indexes() {
        let source = "BYTE ARRAY a(4) BYTE i,b PROC Main() i=1 b=a(i+1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::TAX));
        assert!(output.bytes.contains(&opcode::LDA_ABS_X));
    }

    #[test]
    fn native_pointer_byte_array_reads_accept_dynamic_indexes() {
        let source = "BYTE FUNC Read(BYTE ARRAY a, BYTE i) RETURN(a(i+1))";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ADC_ZP));
        assert!(output.bytes.contains(&opcode::LDA_IZY));
    }

    #[test]
    fn native_inline_byte_array_writes_accept_dynamic_indexes() {
        let source = "BYTE ARRAY a(4) BYTE i PROC Main() i=1 a(i+1)=7 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::TAX));
        assert!(output.bytes.contains(&opcode::STA_ABS_X));
    }

    #[test]
    fn native_dynamic_array_writes_preserve_call_indexes() {
        let source = "BYTE FUNC Next(BYTE n) RETURN(n+1) PROC Fill(CARD ARRAY words BYTE idx) words(Next(idx))=$1234 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_inline_byte_array_compounds_accept_dynamic_indexes() {
        let source = "BYTE ARRAY tagged(2) BYTE winnum PROC Main() tagged(winnum)==+1 tagged(winnum)==-1 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_ABS_X));
        assert!(output.bytes.contains(&opcode::ADC_IMM));
        assert!(output.bytes.contains(&opcode::SBC_IMM));
        assert!(output.bytes.contains(&opcode::STA_ABS_X));
    }

    #[test]
    fn native_inline_byte_array_compounds_accept_logic_ops() {
        let source = "BYTE ARRAY tagged(2) BYTE winnum PROC Main() tagged(winnum)==!$20 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_ABS_X));
        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::STA_ABS_X));
    }

    #[test]
    fn native_pointer_byte_array_writes_accept_dynamic_indexes() {
        let source = "PROC Write(BYTE ARRAY a, BYTE i) a(i+1)=7 RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ADC_ZP));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_pointer_byte_array_writes_preserve_address_for_call_results() {
        let source = "BYTE POINTER p BYTE i BYTE FUNC Internal(BYTE ch) RETURN(ch) PROC Main() p(i+1)=Internal(7) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_conditions_accept_dynamic_inline_byte_array_left_operands() {
        let source = "BYTE ARRAY a(4) BYTE i,hit PROC Main() i=1 IF a(i+1)=7 THEN hit=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::TAX));
        assert!(output.bytes.contains(&opcode::LDA_ABS_X));
        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_accept_dynamic_pointer_byte_array_left_operands() {
        let source = "PROC Check(BYTE ARRAY a, BYTE i, BYTE ch) IF a(i+1)=ch THEN i=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ADC_ZP));
        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::EOR_ABS));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_accept_byte_pointer_deref_left_operands() {
        let source = "PROC Check(BYTE POINTER p, BYTE max) IF p^<max THEN max=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::CMP_ABS));
        assert!(output.bytes.contains(&opcode::BCC_REL));
    }

    #[test]
    fn native_conditions_accept_signed_word_pointer_deref_zero_ordering() {
        let source = "PROC Check(INT POINTER p BYTE out) IF p^>0 THEN out=1 ELSEIF p^<0 THEN out=2 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::AND_IMM));
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::AND_IMM, 0x80])
        );
    }

    #[test]
    fn native_word_deref_assignment_accepts_unary_negation() {
        let source = "PROC Check(INT POINTER p) p^=-p^ RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::SBC_ZP) || output.bytes.contains(&opcode::SBC_ABS));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_if_supports_elseif_chains() {
        let source =
            "BYTE a,out PROC Main() IF a=1 THEN out=1 ELSEIF a=2 THEN out=2 ELSE out=3 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        let beq_count = output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::BEQ_REL)
            .count();
        assert!(beq_count >= 2);
    }

    #[test]
    fn native_conditions_support_logical_or_and_and() {
        let source = "BYTE a,b,out PROC Main() IF a=1 OR a=2 THEN out=1 FI IF a#0 AND b#0 THEN out=2 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        let beq_count = output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::BEQ_REL)
            .count();
        let bne_count = output
            .bytes
            .iter()
            .filter(|byte| **byte == opcode::BNE_REL)
            .count();
        assert!(beq_count >= 2);
        assert!(bne_count >= 2);
    }

    #[test]
    fn native_nonzero_bitwise_conditions_materialize_before_branch() {
        let source = "BYTE skstat=$D20F,out PROC Main() WHILE skstat&$04 DO out==+1 OD RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(5).any(|bytes| {
            matches!(bytes, [opcode::LDA_ABS, 0x0F, 0xD2, opcode::AND_IMM, 0x04])
        }));
        assert!(
            !output
                .bytes
                .windows(4)
                .any(|bytes| matches!(bytes, [opcode::LDA_ABS, 0x0F, 0xD2, opcode::BNE_REL]))
        );
    }

    #[test]
    fn native_conditions_support_word_equality() {
        let source =
            "CARD a,b BYTE out PROC Main() IF a=b THEN out=1 FI IF a#b THEN out=2 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_ABS));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
        assert!(output.bytes.contains(&opcode::BNE_REL));
    }

    #[test]
    fn native_conditions_support_record_field_pointer_equality() {
        let source = "TYPE Node=[CARD next] Node rec BYTE out PROC Check(Node POINTER cur) IF cur.next=0 THEN out=1 FI RETURN PROC Main() Check(rec) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
        assert!(output.bytes.contains(&opcode::BNE_REL));
    }

    #[test]
    fn native_conditions_support_unsigned_word_record_field_ordering() {
        let source = "TYPE Node=[CARD size] Node rec BYTE out PROC Check(Node POINTER n) IF n.size>$0100 THEN out=1 FI RETURN PROC Main() Check(rec) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::CMP_IMM));
        assert!(output.bytes.contains(&opcode::BCC_REL));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_support_byte_ordering_ops() {
        let source = "BYTE a,b,out PROC Main() IF a>b THEN out=1 FI IF a<=b THEN out=2 FI IF a>=b THEN out=3 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::BCC_REL));
        assert!(output.bytes.contains(&opcode::BCS_REL));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_support_ordering_with_deref_operands() {
        let source = "BYTE POINTER s,t BYTE out PROC Main() IF s^>t^ THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::CMP_ZP, runtime_zp::AFCUR.address(),])
        );
    }

    #[test]
    fn native_conditions_support_pointer_equality() {
        let source = "BYTE POINTER p,q BYTE out PROC Main() IF p=q THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_ABS));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_support_parameter_pointer_equality() {
        let source = "BYTE out PROC Check(BYTE POINTER p,q) IF p=q THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_ABS));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_support_pointer_equality_inside_loops() {
        let source = "CARD FUNC Next(BYTE POINTER menu) RETURN(menu+menu^+5) BYTE FUNC Ord(BYTE POINTER menu,item) BYTE c c=0 WHILE menu^ DO IF menu=item THEN EXIT FI c==+1 menu=Next(menu) OD RETURN(c)";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::EOR_ABS));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_support_word_nonzero_call_results() {
        let source =
            "CARD FUNC Find() RETURN($1234) BYTE out PROC Main() IF Find() THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::BNE_REL));
    }

    #[test]
    fn native_conditions_support_word_call_result_equality() {
        let source = "INT FUNC Clamp(INT value) RETURN(value) BYTE out PROC Main() IF Clamp(4)=4 THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_accept_byte_call_results_with_pointer_args() {
        let source = "BYTE FUNC Key(BYTE POINTER menu) RETURN(menu^) BYTE out PROC Check(BYTE POINTER menu BYTE ch) IF Key(menu)=ch THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_conditions_keep_byte_call_results_byte_sized_with_card_literals() {
        let source =
            "BYTE FUNC Value() RETURN(10) BYTE out PROC Main() IF Value()=$0A THEN out=1 FI RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::JSR_ABS));
        assert!(output.bytes.contains(&opcode::EOR_IMM));
        assert!(output.bytes.contains(&opcode::BEQ_REL));
    }

    #[test]
    fn native_array_parameters_can_be_forwarded_to_array_parameters() {
        let source = "PROC Print(BYTE ARRAY s) RETURN PROC PrintE(BYTE ARRAY s) Print(s) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDX_ABS));
        assert!(output.bytes.contains(&opcode::LDA_ABS));
        assert!(output.bytes.contains(&opcode::JSR_ABS));
    }

    #[test]
    fn native_local_array_decay_can_assign_base_address_to_word() {
        let source = "BYTE ARRAY name(4) CARD r PROC Main() r=name RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IMM));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_address_of_large_absolute_array_assigns_backing_address() {
        let source = "BYTE ARRAY allocbuf($800)=$2000 CARD POINTER allocp PROC Main() allocp=CARD POINTER(@allocbuf) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(&output.bytes[..4], &[0x00, 0x20, 0x00, 0x20]);
        assert!(output.bytes.windows(12).any(|bytes| {
            bytes
                == [
                    opcode::LDA_ABS,
                    0x01,
                    0x30,
                    opcode::STA_ABS,
                    0x05,
                    0x30,
                    opcode::LDA_ABS,
                    0x00,
                    0x30,
                    opcode::STA_ABS,
                    0x04,
                    0x30,
                ]
        }));
    }

    #[test]
    fn native_large_absolute_array_decay_assigns_backing_address() {
        let source = "BYTE ARRAY allocbuf($800)=$2000 CARD p PROC Main() p=allocbuf RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert_eq!(&output.bytes[..4], &[0x00, 0x20, 0x00, 0x20]);
        assert!(output.bytes.windows(12).any(|bytes| {
            bytes
                == [
                    opcode::LDA_ABS,
                    0x01,
                    0x30,
                    opcode::STA_ABS,
                    0x05,
                    0x30,
                    opcode::LDA_ABS,
                    0x00,
                    0x30,
                    opcode::STA_ABS,
                    0x04,
                    0x30,
                ]
        }));
    }

    #[test]
    fn native_symbolic_fixed_array_initializer_uses_pointer_descriptor() {
        let source = r#"
            BYTE c
            PROC Jmp=*()
            [<Target >Target]
            PROC Target() RETURN
            PROC Main()
              CARD ARRAY adr(9)=Jmp
              CARD go
              go=adr(c)
            RETURN
        "#;
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        let jmp = routine_address(&output, "Jmp");
        let main = routine_address(&output, "Main");
        let descriptor = usize::from(main - output.origin - 6);
        assert_eq!(
            &output.bytes[descriptor..descriptor + 4],
            &[
                (jmp & 0x00FF) as u8,
                (jmp >> 8) as u8,
                (jmp & 0x00FF) as u8,
                (jmp >> 8) as u8,
            ]
        );
        assert!(output.bytes.contains(&opcode::LDA_IZY));
    }

    #[test]
    fn native_inline_word_array_reads_accept_dynamic_indexes() {
        let source = "CARD ARRAY a=[1 2 3 4] BYTE i CARD w PROC Main() i=1 w=a(i+1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::TAX));
        assert!(output.bytes.contains(&opcode::LDA_ABS_X));
    }

    #[test]
    fn native_pointer_word_array_reads_accept_dynamic_indexes() {
        let source = "CARD FUNC Read(CARD ARRAY a, BYTE i) RETURN(a(i+1))";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::LDA_IZY));
    }

    #[test]
    fn native_pointer_index_word_reads_accept_dynamic_indexes() {
        let source = "CARD POINTER p BYTE i CARD w PROC Main() i=1 w=p(i) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::STA_ABS));
    }

    #[test]
    fn native_pointer_index_word_stores_accept_pointer_index_reads() {
        let source = "PROC Copy(CARD POINTER dst, src) dst(1)=src(1) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDA_IZY));
        assert!(output.bytes.contains(&opcode::PHA));
        assert!(output.bytes.contains(&opcode::PLA));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_inline_word_array_writes_accept_dynamic_indexes() {
        let source = "CARD ARRAY a=[1 2 3 4] BYTE i CARD w PROC Main() i=1 w=$1234 a(i+1)=w RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::TAX));
        assert!(output.bytes.contains(&opcode::STA_ABS_X));
    }

    #[test]
    fn native_pointer_word_array_writes_accept_dynamic_indexes() {
        let source = "PROC Write(CARD ARRAY a, BYTE i, CARD w) a(i+1)=w RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::ASL_A));
        assert!(output.bytes.contains(&opcode::STA_IZY));
    }

    #[test]
    fn native_classifier_recognizes_literal_storage_and_computed_values() {
        let source = "BYTE a,b PROC Main() b=7 a=b+1 RETURN";
        with_prepared_native_emitter(source, |semir, emitter| {
            let routine = first_routine(semir);
            let literal = assignment_value(routine, 0);
            let computed = assignment_value(routine, 1);
            let target = assignment_target(routine, 1);
            let classifier = emitter.classifier();

            assert_eq!(
                classifier.value_shape(literal).unwrap(),
                NativeValueShape::Literal { value: 7, width: 1 }
            );
            assert!(matches!(
                classifier.lvalue_shape(target).unwrap(),
                NativeValueShape::Storage(NativeResolvedSlot { width: 1, .. })
            ));
            assert_eq!(
                classifier.value_shape(computed).unwrap(),
                NativeValueShape::Computed { width: Some(1) }
            );
        });
    }

    #[test]
    fn native_classifier_recognizes_array_decay_address_values() {
        let source = "BYTE ARRAY name(4) CARD r PROC Main(BYTE ARRAY s) r=name r=s RETURN";
        with_prepared_native_emitter(source, |semir, emitter| {
            let routine = first_routine(semir);
            let global_decay = assignment_value(routine, 0);
            let parameter_decay = assignment_value(routine, 1);
            let classifier = emitter.classifier();

            assert!(matches!(
                classifier.value_shape(global_decay).unwrap(),
                NativeValueShape::Address(NativeAddressShape {
                    kind: NativeAddressKind::StorageBase,
                    source,
                    address: Some(0x3000),
                }) if source == "name"
            ));
            assert!(matches!(
                classifier.address_shape(global_decay).unwrap(),
                Some(NativeAddressShape {
                    kind: NativeAddressKind::StorageBase,
                    source,
                    address: Some(0x3000),
                }) if source == "name"
            ));
            assert!(matches!(
                classifier.value_shape(parameter_decay).unwrap(),
                NativeValueShape::Address(NativeAddressShape {
                    kind: NativeAddressKind::StoragePointer,
                    source,
                    address: Some(_),
                }) if source == "s"
            ));
        });
    }

    #[test]
    fn native_classifier_recognizes_indexed_and_deref_values() {
        let source =
            "BYTE ARRAY arr(4) BYTE POINTER p BYTE i,b PROC Main() b=arr(i) b=p(i) b=p^ RETURN";
        with_prepared_native_emitter(source, |semir, emitter| {
            let routine = first_routine(semir);
            let classifier = emitter.classifier();

            assert_eq!(
                classifier
                    .value_shape(assignment_value(routine, 0))
                    .unwrap(),
                NativeValueShape::Indexed(NativeIndexedShape {
                    base: "arr".to_string(),
                    index: "i".to_string(),
                    element_width: 1,
                    storage: NativeIndexedStorage::Inline,
                })
            );
            assert_eq!(
                classifier
                    .value_shape(assignment_value(routine, 1))
                    .unwrap(),
                NativeValueShape::Indexed(NativeIndexedShape {
                    base: "p".to_string(),
                    index: "i".to_string(),
                    element_width: 1,
                    storage: NativeIndexedStorage::PointerValue,
                })
            );
            assert_eq!(
                classifier
                    .value_shape(assignment_value(routine, 2))
                    .unwrap(),
                NativeValueShape::Deref {
                    pointer: "p".to_string(),
                    width: 1,
                }
            );
        });
    }

    #[test]
    fn native_classifier_projects_full_byte_right_shifts() {
        let source = "CARD h BYTE out PROC Main() out=h RSH 8 RETURN";
        with_prepared_native_emitter(source, |semir, emitter| {
            let routine = first_routine(semir);
            let value = assignment_value(routine, 0);

            assert_eq!(
                emitter.classifier().value_byte_source(value, 0).unwrap(),
                Some(NativeByteSource::Storage { address: 0x3001 })
            );
            assert_eq!(
                emitter.classifier().value_byte_source(value, 1).unwrap(),
                Some(NativeByteSource::Immediate(0))
            );
        });
    }

    #[test]
    fn native_classifier_recognizes_call_results() {
        let source = "BYTE FUNC F() RETURN(1) PROC Main() BYTE b b=F() RETURN";
        with_prepared_native_emitter(source, |semir, emitter| {
            let routine = semir
                .modules
                .iter()
                .flat_map(|module| &module.items)
                .filter_map(|item| match item {
                    SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                    _ => None,
                })
                .next()
                .expect("Main routine");

            assert_eq!(
                emitter
                    .classifier()
                    .value_shape(assignment_value(routine, 0))
                    .unwrap(),
                NativeValueShape::CallResult {
                    callee: "F".to_string(),
                    width: Some(1),
                }
            );
        });
    }

    #[test]
    fn native_call_arg_width_and_constants_use_classifier_shapes() {
        let source = "BYTE ARRAY inline(4) BYTE ARRAY big(300) PROC Take(BYTE ARRAY x) RETURN PROC Main(BYTE ARRAY s) Take(inline) Take(big) Take(s) RETURN";
        with_prepared_native_emitter(source, |semir, emitter| {
            let routine = semir
                .modules
                .iter()
                .flat_map(|module| &module.items)
                .filter_map(|item| match item {
                    SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                    _ => None,
                })
                .next()
                .expect("Main routine");
            let unknown_callee = SymbolId(usize::MAX);
            let inline = call_arg(routine, 0, 0);
            let descriptor = call_arg(routine, 1, 0);
            let parameter = call_arg(routine, 2, 0);

            assert_eq!(
                emitter
                    .call_arg_width(Some(unknown_callee), 0, inline)
                    .unwrap(),
                2
            );
            assert_eq!(emitter.classifier().value_width(inline).unwrap(), 2);
            assert_eq!(
                emitter.classifier().value_byte_source(inline, 0).unwrap(),
                Some(NativeByteSource::Immediate(0x00))
            );
            assert_eq!(
                emitter.classifier().value_byte_source(inline, 1).unwrap(),
                Some(NativeByteSource::Immediate(0x30))
            );
            assert_eq!(
                emitter.classifier().compare_byte_source(inline, 1).unwrap(),
                Some(NativeByteSource::Immediate(0x30))
            );
            assert_eq!(
                emitter
                    .classifier()
                    .word_source(inline, NativeByteSourceMode::Exact)
                    .unwrap(),
                Some(NativeWordSource {
                    low: NativeByteSource::Immediate(0x00),
                    high: NativeByteSource::Immediate(0x30),
                })
            );
            assert_eq!(
                emitter
                    .call_arg_width(Some(unknown_callee), 0, descriptor)
                    .unwrap(),
                2
            );
            assert!(matches!(
                emitter
                    .classifier()
                    .value_byte_source(descriptor, 0)
                    .unwrap(),
                Some(NativeByteSource::Storage { .. })
            ));
            assert_eq!(
                emitter
                    .call_arg_width(Some(unknown_callee), 0, parameter)
                    .unwrap(),
                2
            );
            assert!(matches!(
                emitter
                    .classifier()
                    .value_byte_source(parameter, 0)
                    .unwrap(),
                Some(NativeByteSource::Storage { .. })
            ));
        });
    }

    #[test]
    fn native_word_call_args_accept_byte_products() {
        let source = "BYTE dx,dy CARD out CARD FUNC Echo(CARD n) RETURN(n) PROC Main() dx=2 dy=3 out=Echo((dx+1)*(dy+1)) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.windows(3).any(|bytes| {
            bytes
                == [
                    opcode::JSR_ABS,
                    runtime_helper::CARTRIDGE_MUL.low(),
                    runtime_helper::CARTRIDGE_MUL.high(),
                ]
        }));
    }

    #[test]
    fn native_word_call_args_stage_dynamic_word_array_reads() {
        let source = "CARD ARRAY names(4) BYTE i PROC Print(CARD p) RETURN PROC Main() i=1 Print(names(i)) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDX_ZP, runtime_zp::ARGS.offset(1).address(),])
        );
        assert!(
            output
                .bytes
                .windows(2)
                .any(|bytes| bytes == [opcode::LDA_ZP, runtime_zp::ARGS.address(),])
        );
    }

    #[test]
    fn native_word_call_args_accept_constant_word_array_reads() {
        let source = "INT ARRAY nums(2) INT FUNC Neg(INT n) RETURN(0-n) PROC Main() nums(1)=Neg(nums(0)) RETURN";
        let semir = lower_source(source);
        let output =
            generate_native_profile_with_origin(&semir, 0x3000, CodegenProfile::Modern).unwrap();

        assert!(output.bytes.contains(&opcode::LDX_ABS) || output.bytes.contains(&opcode::LDX_ZP));
        assert!(output.bytes.contains(&opcode::LDA_ABS) || output.bytes.contains(&opcode::LDA_ZP));
        assert!(output.bytes.contains(&opcode::JSR_ABS));
    }

    #[test]
    fn semir_native_uses_tracked_emitter_guardrail() {
        let source = include_str!("semir_native.rs");
        let emission_source = include_str!("semir_native/native_emit.rs");

        assert!(
            source.contains("emitter: NativeTrackedEmitter"),
            "semIR native backend should own the tracked emitter facade"
        );
        assert!(
            source.contains("NativeTrackedEmitter::with_origin(model.origin)"),
            "semIR native backend should construct the tracked emitter facade"
        );
        assert!(
            !source.contains(concat!("emitter", ": Emitter")),
            "semIR native backend must not own the raw byte emitter directly"
        );
        assert!(
            !source.contains(concat!(
                "emitter",
                ": Emitter",
                "::with_origin(model.origin)"
            )),
            "semIR native backend must not construct the raw byte emitter directly"
        );
        assert!(
            source.contains("mod native_emit;"),
            "semIR native backend should keep concrete emission helpers in their own module"
        );
        assert!(
            emission_source.contains("pub(super) fn emit_lda_addr")
                && emission_source.contains("pub(super) fn emit_sta_addr")
                && emission_source.contains("pub(super) fn emit_pha")
                && emission_source.contains("pub(super) fn emit_dey")
                && emission_source.contains("pub(super) fn emit_beq_label")
                && emission_source.contains("pub(super) fn emit_jmp_label")
                && emission_source.contains("pub(super) fn emit_jmp_addr")
                && emission_source.contains("pub(super) fn emit_jsr_addr")
                && emission_source.contains("pub(super) fn emit_raw_u8")
                && emission_source.contains("pub(super) fn emit_lda_imm")
                && emission_source.contains("pub(super) fn emit_adc_imm")
                && emission_source.contains("pub(super) fn emit_sbc_element_addr")
                && emission_source.contains("pub(super) fn emit_rts")
                && emission_source.contains("pub(super) fn emit_tax")
                && emission_source.contains("pub(super) fn emit_tay")
                && emission_source.contains("pub(super) fn emit_and_addr")
                && emission_source.contains("pub(super) fn ensure_y_zero"),
            "native emission module should own concrete address, call, label, raw, and Y-state helpers"
        );
        assert!(
            !source.contains(concat!("self.", "emitter.", "emit_")),
            "high-level SemIR native lowering should route concrete emission through native_emit.rs"
        );
    }

    fn with_prepared_native_emitter<R>(
        source: &str,
        f: impl FnOnce(&SemProgram, &mut SemIrNativeEmitter<'_, '_>) -> R,
    ) -> R {
        let semir = lower_source(source);
        let model = SemIrReadModel::new(&semir, 0x3000, CodegenProfile::Modern);
        let mut emitter = SemIrNativeEmitter::new(&model);
        emitter.emit_global_storage().unwrap();
        for routine in &model.routines {
            emitter.emit_param_storage(routine.routine).unwrap();
            emitter.emit_local_storage(routine.routine).unwrap();
        }
        f(&semir, &mut emitter)
    }

    fn first_routine(semir: &SemProgram) -> &SemRoutine {
        semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("routine")
    }

    fn routine_address(output: &CodegenOutput, name: &str) -> u16 {
        output
            .routine_addresses
            .iter()
            .find(|routine| routine.name.eq_ignore_ascii_case(name))
            .map(|routine| routine.address)
            .unwrap_or_else(|| panic!("missing routine address for {name}"))
    }

    fn assignment_value(routine: &SemRoutine, index: usize) -> &SemExpr {
        let SemStmt::Assign { value, .. } = &routine.body[index] else {
            panic!("expected assignment value at index {index}");
        };
        value
    }

    fn assignment_target(routine: &SemRoutine, index: usize) -> &SemLValue {
        let SemStmt::Assign { target, .. } = &routine.body[index] else {
            panic!("expected assignment target at index {index}");
        };
        target
    }

    fn call_arg(routine: &SemRoutine, stmt_index: usize, arg_index: usize) -> &SemExpr {
        let SemStmt::Call { call, .. } = &routine.body[stmt_index] else {
            panic!("expected call at index {stmt_index}");
        };
        &call.args[arg_index]
    }

    fn lower_source(source: &str) -> SemProgram {
        let tokens = tokenize(source).unwrap();
        let program = parse(&tokens).unwrap();
        let model = analyze(&program).unwrap();
        ir::lower_program(&program, &model)
    }
}
