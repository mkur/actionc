use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::resident::{ResidentVariableKind, ResidentVariableStorage, resident_variable};
use crate::source::{Span, source_char_byte};

const DATA_BASE: u16 = 0x0600;
pub const CODE_ORIGIN: u16 = 0x3000;
pub const RUNAD: u16 = 0x02E2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenOutput {
    pub bytes: Vec<u8>,
    pub origin: u16,
    pub run_address: u16,
    pub skipped_ranges: Vec<SkippedRange>,
    pub routine_addresses: Vec<RoutineAddress>,
    pub optimizations: Vec<CodegenOptimization>,
    pub proofs: Vec<CodegenProof>,
    pub proof_attempts: Vec<CodegenProofAttempt>,
    pub map: CodegenMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenMap {
    pub origin: u16,
    pub run_address: u16,
    pub skipped_ranges: Vec<SkippedRange>,
    pub routine_addresses: Vec<RoutineAddress>,
    pub routine_ranges: Vec<RoutineRange>,
    pub routine_signatures: Vec<CodegenRoutineSignature>,
    pub storage_symbols: Vec<CodegenStorageSymbol>,
    pub source_ranges: Vec<CodegenSourceRange>,
    pub routine_effects: Vec<CodegenRoutineEffect>,
    pub machine_blocks: Vec<CodegenMachineBlockAnalysis>,
    pub optimizations: Vec<CodegenOptimization>,
    pub proofs: Vec<CodegenProof>,
    pub proof_attempts: Vec<CodegenProofAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenRoutineEffect {
    pub routine: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenMachineBlockAnalysis {
    pub routine: Option<String>,
    pub source_span: Span,
    pub address: u16,
    pub trusted: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenProof {
    pub routine: Option<String>,
    pub source_span: Span,
    pub address: Option<u16>,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenProofAttempt {
    pub routine: Option<String>,
    pub source_span: Span,
    pub address: Option<u16>,
    pub kind: String,
    pub accepted: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineAddress {
    pub name: String,
    pub address: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRange {
    pub name: String,
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenRoutineSignature {
    pub name: String,
    pub kind: String,
    pub params: Vec<CodegenRoutineParam>,
    pub return_type: Option<String>,
    pub return_width: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenRoutineParam {
    pub name: String,
    pub type_name: String,
    pub width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedRange {
    pub start: u16,
    pub len: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenStorageSymbol {
    pub name: String,
    pub scope: CodegenSymbolScope,
    pub kind: CodegenSymbolKind,
    pub address: u16,
    pub size: u16,
    pub address_space: CodegenAddressSpace,
    pub pointee_size: Option<u16>,
    pub array: Option<CodegenArrayStorage>,
    pub signed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenSourceRange {
    pub kind: CodegenSourceRangeKind,
    pub name: Option<String>,
    pub source_span: Span,
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenSourceRangeKind {
    Routine,
    Statement,
    Expression,
    Declaration,
    StorageInitializer,
    MachineBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenSymbolScope {
    Global,
    Routine(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenSymbolKind {
    Parameter,
    Local,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenAddressSpace {
    Absolute,
    AbsoluteX,
    ZeroPage,
    IndirectIndexedY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenArrayStorage {
    Inline,
    Pointer,
    Descriptor,
}

#[derive(Debug, Default)]
struct RoutineAllocation {
    symbols: HashMap<String, StorageSlot>,
    initializers: Vec<StorageInit>,
    array_backings: Vec<ArrayBacking>,
    machine_symbol_addresses: HashMap<String, MachineSymbolAddress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoutineEntryPlan {
    kind: RoutineEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutineEntryKind {
    Direct,
    Trampoline(RoutineTrampolineReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutineTrampolineReason {
    CompatibleProfile,
    RetargetableRoutine,
    UnprovenBoundary,
}

impl RoutineEntryPlan {
    fn direct() -> Self {
        Self {
            kind: RoutineEntryKind::Direct,
        }
    }

    fn trampoline(reason: RoutineTrampolineReason) -> Self {
        Self {
            kind: RoutineEntryKind::Trampoline(reason),
        }
    }

    fn is_direct(self) -> bool {
        self.kind == RoutineEntryKind::Direct
    }
}

mod storage;
use storage::*;

mod data;
use data::*;
pub use storage::{
    Absolute, AbsoluteX, Immediate, IndexedIndirectX, IndirectIndexedY, ZeroPage, ZeroPageX,
    runtime_zp,
};

mod model;
use model::*;

mod temp;
#[cfg(test)]
use temp::*;

mod proof;
use proof::*;

mod analysis;
use analysis::*;

mod compat;
use compat::*;

mod runtime;
pub use runtime::runtime_helper;
use runtime::*;

mod driver;
pub use driver::{
    generate, generate_compatible_with_origin, generate_profile_with_origin,
    generate_semir_native_profile_with_origin, generate_semir_profile_with_origin,
    generate_with_origin,
};

mod semir;

mod semir_native;

mod guards;
use guards::*;

mod call;
use call::*;

mod machine;
use machine::*;

mod branch;
use branch::*;

mod arith;

mod array;

mod copy;

mod lvalue;

mod routine;

mod program;

mod output;
use output::*;
pub use output::{format_hex, format_load_file};

mod slot;

mod stmt;

mod assign;

mod emitter;
pub use emitter::{
    AddressingMode, DisassembledInstruction, Emitter, disassemble_with_origin,
    disassemble_with_origin_and_inline_jsr_data, format_listing, format_listing_with_origin,
    opcode,
};
use emitter::{Patch, PatchKind, decode_instruction};

pub(crate) fn decode_6502_opcode(opcode: u8) -> Option<(&'static str, AddressingMode, usize)> {
    let instruction = decode_instruction(opcode)?;
    Some((instruction.mnemonic, instruction.mode, instruction.len))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodegenProfile {
    #[default]
    Compat,
    Modern,
}

impl CodegenProfile {
    fn enables_modern_optimizations(self) -> bool {
        matches!(self, Self::Modern)
    }
}

mod state;
use state::*;

mod native_state;

pub(crate) mod native_emitter;
use native_emitter::*;

fn ranges_overlap(left_address: u16, left_size: u16, right_address: u16, right_size: u16) -> bool {
    let left_end = left_address.saturating_add(left_size.max(1));
    let right_end = right_address.saturating_add(right_size.max(1));
    left_address < right_end && right_address < left_end
}

fn slot_overlaps_zero_page(slot: StorageSlot, zero_page: ZeroPage, size: u16) -> bool {
    slot.space == AddressSpace::ZeroPage
        && ranges_overlap(
            slot.address,
            slot.size,
            u16::from(zero_page.address()),
            size,
        )
}

fn effects_preserve_slot_byte(effects: RoutineEffects, slot: StorageSlot, byte_index: u16) -> bool {
    match slot.space {
        AddressSpace::ZeroPage => !effects.writes_zero_page(slot.zero_page_byte(byte_index)),
        AddressSpace::Absolute => !effects.writes_absolute_range(slot.byte_address(byte_index), 1),
        AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => false,
    }
}

fn slot_dependency(slot: StorageSlot) -> PreparedDependency {
    PreparedDependency {
        address: slot.address,
        size: slot_accessible_byte_size(slot),
    }
}

fn slot_load_instruction_len(slot: StorageSlot) -> i16 {
    match slot.space {
        AddressSpace::ZeroPage | AddressSpace::IndirectIndexedY => 2,
        AddressSpace::Absolute | AddressSpace::AbsoluteX => 3,
    }
}

fn compatible_set_storage_width(slot: StorageSlot) -> u16 {
    match slot.array {
        Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) => 2,
        Some(ArrayStorage::Inline) => slot.size.min(2),
        None => {
            if slot.pointee_size.is_some() {
                2
            } else {
                slot.size.min(2)
            }
        }
    }
}

fn absolute_zero_page_alias(absolute: Absolute) -> Option<ZeroPage> {
    let address = absolute.address();
    (address < 0x100).then(|| ZeroPage::new(address as u8))
}

impl Generator {
    pub(super) fn next_label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}:{}", self.label_counter);
        self.label_counter += 1;
        label
    }

    pub(super) fn constant_u16(&self, expr: &Expr) -> Option<u16> {
        constant_u16_with_defines(expr, &self.numeric_defines)
    }

    pub(super) fn lookup_slot(&self, name: &str) -> Option<StorageSlot> {
        self.local_symbols
            .get(&normalize_name(name))
            .copied()
            .or_else(|| self.layout.lookup(name))
            .or_else(|| builtin_storage_slot(name))
    }

    pub(super) fn annotation_zero_page_symbol_range(
        &mut self,
        name: &str,
        routine_name: &str,
        span: Span,
    ) -> Option<AnnotationZeroPageRange> {
        let Some(slot) = self.lookup_slot(name) else {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!("unknown annotation zero-page symbol `{name}` in `{routine_name}`"),
            ));
            return None;
        };
        if slot.space != AddressSpace::ZeroPage || slot.address.saturating_add(slot.size) > 0x0100 {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!("annotation symbol `{name}` in `{routine_name}` is not zero-page storage"),
            ));
            return None;
        }
        Some(AnnotationZeroPageRange {
            start: slot.address as u8,
            end: slot.address.wrapping_add(slot.size.saturating_sub(1)) as u8,
        })
    }

    pub(super) fn annotation_address_symbol_range(
        &mut self,
        name: &str,
        routine_name: &str,
        span: Span,
    ) -> Option<AnnotationAddressRange> {
        let Some(slot) = self.lookup_slot(name) else {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!("unknown annotation address symbol `{name}` in `{routine_name}`"),
            ));
            return None;
        };
        Some(AnnotationAddressRange {
            start: slot.address,
            end: slot.address.wrapping_add(slot.size.saturating_sub(1)),
        })
    }

    pub(super) fn routine_effects_from_annotations(
        &mut self,
        mut effects: RoutineEffects,
        annotations: &[ActioncAnnotation],
        routine_name: &str,
        span: Span,
    ) -> RoutineEffects {
        if !effects.known
            && annotations.iter().any(|annotation| {
                matches!(
                    annotation,
                    ActioncAnnotation::Preserves { .. }
                        | ActioncAnnotation::Clobbers { .. }
                        | ActioncAnnotation::Writes { .. }
                )
            })
        {
            effects = RoutineEffects::known_empty();
        }
        for annotation in annotations {
            match annotation {
                ActioncAnnotation::Preserves {
                    registers,
                    zero_page,
                } => {
                    for register in registers.iter() {
                        effects.preserve_register(register);
                    }
                    for range in zero_page.ranges.iter().flatten().copied().chain(
                        zero_page.symbols.iter().filter_map(|symbol| {
                            self.annotation_zero_page_symbol_range(symbol, routine_name, span)
                        }),
                    ) {
                        for address in range.start..=range.end {
                            effects.clear_zero_page_write(ZeroPage::new(address));
                        }
                    }
                }
                ActioncAnnotation::Clobbers {
                    registers,
                    zero_page,
                } => {
                    for register in registers.iter() {
                        effects.clobber_register(register);
                    }
                    for range in zero_page.ranges.iter().flatten().copied().chain(
                        zero_page.symbols.iter().filter_map(|symbol| {
                            self.annotation_zero_page_symbol_range(symbol, routine_name, span)
                        }),
                    ) {
                        for address in range.start..=range.end {
                            effects.record_zero_page_write(ZeroPage::new(address));
                        }
                    }
                }
                ActioncAnnotation::Writes { addresses } => {
                    for range in addresses.ranges.iter().flatten().copied().chain(
                        addresses.symbols.iter().filter_map(|symbol| {
                            self.annotation_address_symbol_range(symbol, routine_name, span)
                        }),
                    ) {
                        effects.record_absolute_write(
                            range.start,
                            range.end.wrapping_sub(range.start).wrapping_add(1),
                        );
                    }
                }
                ActioncAnnotation::ReturnsAEqualsA0 | ActioncAnnotation::DebugProfileCompat => {}
            }
        }
        effects
    }

    pub(super) fn record_current_zero_page_write(&mut self, zero_page: ZeroPage) {
        if let Some(effects) = &mut self.current_routine_effects {
            effects.record_zero_page_write(zero_page);
        }
    }

    pub(super) fn record_current_absolute_write(&mut self, address: u16, size: u16) {
        if let Some(effects) = &mut self.current_routine_effects {
            effects.record_absolute_write(address, size);
        }
    }

    pub(super) fn record_current_unknown_absolute_write(&mut self) {
        if let Some(effects) = &mut self.current_routine_effects {
            effects.record_unknown_absolute_write();
        }
        self.label_byte_values.clear();
    }

    pub(super) fn record_current_unknown_effects(&mut self) {
        if let Some(effects) = &mut self.current_routine_effects {
            *effects = RoutineEffects::unknown();
        }
    }

    pub(super) fn merge_current_callee_effects(&mut self, effects: RoutineEffects) {
        if let Some(current) = &mut self.current_routine_effects {
            current.merge(effects);
        }
    }

    pub(super) fn current_absolute_address(&self) -> u16 {
        self.emitter
            .origin
            .wrapping_add(self.emitter.position() as u16)
    }

    pub(super) fn record_source_range(
        &mut self,
        kind: CodegenSourceRangeKind,
        name: Option<String>,
        source_span: Span,
        start: u16,
        end: u16,
    ) {
        if end <= start {
            return;
        }
        self.source_ranges.push(CodegenSourceRange {
            kind,
            name,
            source_span,
            start,
            end,
        });
    }

    pub(super) fn bind_codegen_label(&mut self, label: String, span: Span) {
        self.try_invert_branch_to_label(&label);
        let y_hint = self.label_store_y_hints.remove(&label);
        self.processor.invalidate_accumulator();
        self.processor.invalidate_index_x();
        self.processor.invalidate_all_zp();
        self.processor.invalidate_memory();
        self.processor.set_y_hint(y_hint);
        self.straight_line_store_y = y_hint;
        self.last_label_position = Some(self.emitter.position());
        if let Err(diagnostic) = self.emitter.bind_label(label, span) {
            self.diagnostics.push(diagnostic);
        }
    }

    pub(super) fn bind_codegen_label_preserving_state(
        &mut self,
        label: String,
        span: Span,
        mut processor: ProcessorState,
        straight_line_store_y: Option<u8>,
    ) {
        self.try_invert_branch_to_label(&label);
        let y_hint = self.label_store_y_hints.remove(&label);
        if let Some(y) = y_hint {
            processor.set_y_hint(Some(y));
        }
        self.processor = processor;
        self.straight_line_store_y = y_hint.or(straight_line_store_y);
        self.last_label_position = Some(self.emitter.position());
        if let Err(diagnostic) = self.emitter.bind_label(label, span) {
            self.diagnostics.push(diagnostic);
        }
    }

    pub(super) fn emit_pointer_plus_scaled_byte_index_to_addr(
        &mut self,
        pointer: StorageSlot,
        index: &Expr,
        addr: ZeroPage,
    ) -> bool {
        let Some(size) = pointer.pointee_size else {
            return false;
        };
        if size != 1 && size != 2 {
            return false;
        }
        let Some(index_slot) = self.direct_scalar_slot(index) else {
            return false;
        };
        if index_slot.size != 1 {
            return false;
        }

        if size == 1 {
            self.emit_clc();
            self.emit_lda_slot_byte(pointer, 0);
            self.emit_adc_slot_byte(index_slot, 0);
            self.emit_sta_zero_page(addr);
            self.emit_lda_slot_byte(pointer, 1);
            self.emit_adc_imm(0);
            self.emit_sta_zero_page(addr.offset(1));
            return true;
        }

        self.emit_lda_slot_byte_value_only(index_slot, 0);
        self.emit_asl_a();
        self.emitter.emit_php();
        self.emit_clc();
        self.emit_adc_slot_byte(pointer, 0);
        self.emit_sta_zero_page(addr);
        self.emit_lda_imm(0);
        self.emit_rol_a();
        self.emit_plp();
        self.emit_adc_slot_byte(pointer, 1);
        self.emit_sta_zero_page(addr.offset(1));
        true
    }

    pub(super) fn emit_expr_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        debug_assert_expr_target_slot_shape(expr, slot);
        if self.segment_storage && expr_uses_compatible_runtime_arithmetic(expr) {
            return self.emit_binary_expr_to_slot(expr, slot);
        }

        if let Some(value) = self.constant_u16(expr) {
            self.emit_store_constant(slot, value);
            return true;
        }

        match &expr.kind {
            ExprKind::Cast { expr, .. } => self.emit_expr_to_slot(expr, slot),
            ExprKind::Name(name) => {
                if self.emit_routine_address_to_slot(name, slot, expr.span) {
                    true
                } else {
                    self.emit_copy_expr_to_slot(expr, slot)
                }
            }
            ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Unary {
                op: UnaryOp::Deref | UnaryOp::AddressOf,
                ..
            } => self.emit_copy_expr_to_slot(expr, slot),
            ExprKind::Call { callee, args }
                if self.array_call_slot_size(callee, args).is_some() =>
            {
                self.emit_copy_expr_to_slot(expr, slot)
            }
            ExprKind::Call { callee, args } => {
                if let Some(info) = self.call_routine_info(callee) {
                    let Some(return_slot) = info.return_slot else {
                        return false;
                    };
                    if !self.emit_call(callee, args, expr.span) {
                        return false;
                    }
                    self.emit_copy_call_return_slot_to_slot(return_slot, slot, info.internal_abi())
                } else {
                    let Some(return_slot) = self.call_return_slot(callee) else {
                        return false;
                    };
                    if !self.emit_call(callee, args, expr.span) {
                        return false;
                    }
                    self.emit_copy_slot_to_slot(return_slot, slot)
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Neg,
                ..
            } => self.emit_unary_expr_to_slot(expr, slot),
            ExprKind::Binary {
                op:
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::Lsh
                    | BinaryOp::Rsh,
                ..
            } => self.emit_binary_expr_to_slot(expr, slot),
            _ => false,
        }
    }

    pub(super) fn emit_routine_address_to_slot(
        &mut self,
        name: &str,
        slot: StorageSlot,
        span: Span,
    ) -> bool {
        if !self.segment_storage || slot.size < 2 {
            return false;
        }
        let Some(routine) = self.routines.get(&normalize_name(name)).cloned() else {
            return false;
        };

        if let Some(address) = routine.system_address {
            self.emit_store_constant(slot, address);
            self.processor.set_memory_address_word(slot, address);
        } else {
            self.emit_lda_label_high(routine.label.clone(), span);
            self.emit_sta_slot_byte(slot, 1);
            self.emit_lda_label_low(routine.label, span);
            self.emit_sta_slot_byte(slot, 0);
            self.processor.invalidate_index_y();
            self.straight_line_store_y = None;
        }
        true
    }

    pub(super) fn emit_unary_expr_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        let ExprKind::Unary { op, expr } = &expr.kind else {
            return false;
        };
        match op {
            UnaryOp::Plus => self.emit_expr_to_slot(expr, slot),
            UnaryOp::Neg => self.emit_neg_expr_to_slot(expr, slot),
            UnaryOp::Deref | UnaryOp::AddressOf => false,
        }
    }

    pub(super) fn emit_neg_expr_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        let width = slot.size.min(2);
        if width == 0 {
            return false;
        }

        if self.segment_storage
            && let Some(source) = self.negation_source_slot(expr, slot)
        {
            self.emit_sec();
            self.emit_lda_imm(0);
            self.emit_sbc_slot_byte(source, 0);
            self.emit_sta_slot_byte(slot, 0);

            if width > 1 {
                self.emit_lda_imm(0);
                if source.size > 1 {
                    self.emit_sbc_slot_byte(source, 1);
                } else {
                    self.emit_sbc_imm(0);
                }
                self.emit_sta_slot_byte(slot, 1);
            }
            return true;
        }
        if self.segment_storage && Self::expr_may_prepare_indirect_slot(expr) {
            return false;
        }

        self.emit_sec();
        self.emit_lda_imm(0);
        if !self.emit_sub_simple_byte(expr, 0) {
            return false;
        }
        self.emit_sta_slot_byte(slot, 0);

        if width > 1 {
            self.emit_lda_imm(0);
            if !self.emit_sub_simple_byte(expr, 1) {
                return false;
            }
            self.emit_sta_slot_byte(slot, 1);
        }
        true
    }

    pub(super) fn negation_source_slot(
        &mut self,
        expr: &Expr,
        target: StorageSlot,
    ) -> Option<StorageSlot> {
        let source = self.lvalue_slot(expr)?;
        debug_assert_negation_source_slot(expr, source, target);
        Some(source)
    }

    pub(super) fn emit_copy_expr_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        if self.emit_effective_address_to_slot(expr, slot) {
            return true;
        }

        if self.segment_storage && slot.size == 2 {
            if self.emit_array_pointer_value_to_slot(expr, slot) {
                return true;
            }

            if slot.space == AddressSpace::IndirectIndexedY {
                let source_pointer = if slot.zero_page_byte(0) == runtime_zp::ARRAY_ADDR {
                    runtime_zp::ELEMENT_ADDR
                } else {
                    runtime_zp::ARRAY_ADDR
                };
                if let Some(source) = self.reusable_lvalue_slot_with_pointer(expr, source_pointer) {
                    debug_assert_indirect_slots_do_not_alias(source, slot, "indirect copy");
                    self.emit_lda_slot_byte(source, 1);
                    self.emit_sta_slot_byte(slot, 1);
                    self.emit_lda_slot_byte(source, 0);
                    self.emit_sta_slot_byte(slot, 0);
                    return true;
                }
            }

            if let Some(source) = self.reusable_lvalue_slot(expr)
                && source.space == AddressSpace::IndirectIndexedY
            {
                debug_assert_indirect_slots_do_not_alias(source, slot, "word copy");
                if source.size > 1 {
                    self.emit_lda_slot_byte(source, 1);
                } else {
                    self.emit_lda_imm(0);
                }
                self.emit_sta_slot_byte(slot, 1);
                self.emit_lda_slot_byte(source, 0);
                self.emit_sta_slot_byte(slot, 0);
                return true;
            }

            if !self.emit_load_simple_byte(expr, 1) {
                return false;
            }
            self.emit_sta_slot_byte(slot, 1);
            if !self.emit_load_simple_byte(expr, 0) {
                return false;
            }
            self.emit_sta_slot_byte(slot, 0);
            return true;
        }

        if self.segment_storage
            && slot.space == AddressSpace::IndirectIndexedY
            && slot.zero_page_byte(0) == runtime_zp::ARRAY_ADDR
            && let Some(source) =
                self.constant_descriptor_index_slot_with_pointer(expr, runtime_zp::ELEMENT_ADDR)
        {
            self.emit_lda_slot_byte(source, 0);
            self.emit_sta_slot_byte(slot, 0);
            if slot.size > 1 {
                if source.size > 1 {
                    self.emit_lda_slot_byte(source, 1);
                } else {
                    self.emit_lda_imm(0);
                }
                self.emit_sta_slot_byte(slot, 1);
            }
            return true;
        }

        if self.profile.enables_modern_optimizations()
            && slot.size == 1
            && let Some(value) = self.known_zero_page_scalar_immediate(expr)
        {
            self.emit_store_constant(slot, u16::from(value));
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                Some(expr.span),
                format!(
                    "stored known zero-page value #${value:02X} directly instead of reloading source"
                ),
            );
            return true;
        }

        if self.emit_inline_byte_array_scalar_index_to_slot(expr, slot) {
            return true;
        }

        if self.profile.enables_modern_optimizations()
            && let Some(source) = self.reusable_lvalue_slot(expr)
        {
            return self.emit_copy_slot_to_slot(source, slot);
        }

        if self.profile.enables_modern_optimizations()
            && self.emit_proven_simple_value_to_slot(expr, slot)
        {
            return true;
        }

        if !self.emit_load_simple_byte(expr, 0) {
            return false;
        }
        self.emit_sta_slot_byte(slot, 0);

        if slot.size > 1 {
            if !self.emit_load_simple_byte(expr, 1) {
                return false;
            }
            self.emit_sta_slot_byte(slot, 1);
        }

        true
    }

    pub(super) fn emit_proven_simple_value_to_slot(
        &mut self,
        expr: &Expr,
        target: StorageSlot,
    ) -> bool {
        let proof = self.value_availability_proof(expr);
        if !matches!(
            proof.source,
            ValueAvailabilitySource::Constant | ValueAvailabilitySource::Storage
        ) {
            self.record_codegen_proof_rejection(
                "value-availability",
                expr.span,
                "assignment fallback requires constant or storage source",
            );
            return false;
        }
        let Some(width) = proof.width else {
            self.record_codegen_proof_rejection(
                "value-availability",
                expr.span,
                "assignment source width is unknown",
            );
            return false;
        };
        if target.size > 2 || width > 2 {
            self.record_codegen_proof_rejection(
                "value-availability",
                expr.span,
                format!(
                    "assignment proof fallback only supports up to two bytes, source width {width}, target width {}",
                    target.size
                ),
            );
            return false;
        }
        for byte_index in (0..target.size).rev() {
            if byte_index < width {
                match proof.bytes.get(usize::from(byte_index)).copied().flatten() {
                    Some(ValueByteAvailability::Constant(value)) => {
                        self.emit_lda_imm(value);
                    }
                    Some(ValueByteAvailability::Slot { slot, byte_index })
                    | Some(ValueByteAvailability::PublicReturnSlot { slot, byte_index }) => {
                        self.emit_lda_slot_byte(slot, byte_index);
                    }
                    Some(ValueByteAvailability::Register(RegisterName::A)) => {}
                    Some(ValueByteAvailability::Register(RegisterName::X)) => self.emit_txa(),
                    Some(ValueByteAvailability::Register(RegisterName::Y)) => self.emit_tya(),
                    None => {
                        self.record_codegen_proof_rejection(
                            "value-availability",
                            expr.span,
                            format!("assignment source byte {byte_index} is not available"),
                        );
                        return false;
                    }
                }
            } else {
                self.emit_lda_imm(0);
            }
            self.emit_sta_slot_byte(target, byte_index);
        }
        self.record_codegen_proof(
            "value-availability",
            expr.span,
            "assignment source bytes are available from value proof",
        );
        self.record_modern_optimization(
            CodegenOptimizationKind::RegisterReloadRemoved,
            0,
            Some(expr.span),
            "stored expression through value availability proof",
        );
        true
    }
}

struct Generator {
    emitter: Emitter,
    layout: StorageLayout,
    record_layouts: RecordLayouts,
    routines: HashMap<String, RoutineInfo>,
    callable_pointers: HashMap<String, CallablePointerInfo>,
    numeric_defines: HashMap<String, u16>,
    machine_defines: HashMap<String, Vec<MachineItem>>,
    runtime_helpers: RuntimeHelperTargets,
    routine_assignment_targets: HashSet<String>,
    local_symbols: HashMap<String, StorageSlot>,
    local_callable_pointers: HashMap<String, CallablePointerInfo>,
    storage_symbols: Vec<CodegenStorageSymbol>,
    source_ranges: Vec<CodegenSourceRange>,
    current_return_slot: Option<StorageSlot>,
    diagnostics: Vec<Diagnostic>,
    label_counter: usize,
    exit_labels: Vec<String>,
    profile: CodegenProfile,
    segment_storage: bool,
    processor: ProcessorState,
    straight_line_store_y: Option<u8>,
    y_constant_store_lookahead: Option<u8>,
    label_store_y_hints: HashMap<String, u8>,
    label_byte_values: HashMap<String, ValueFact>,
    last_label_position: Option<usize>,
    compatible_cursor: Option<u16>,
    skipped_ranges: Vec<SkippedRange>,
    last_routine_label: Option<String>,
    last_routine_ended_with_rts: bool,
    routine_addresses: Vec<RoutineAddress>,
    routine_ranges: Vec<RoutineRange>,
    routine_signatures: Vec<CodegenRoutineSignature>,
    current_routine_effects: Option<RoutineEffects>,
    current_routine_has_effect_contract: bool,
    current_inferred_routine_facts: Option<InferredRoutineFacts>,
    current_modern_routine_layout: ModernRoutineLayout,
    preserve_modern_routine_layout: bool,
    machine_blocks: Vec<CodegenMachineBlockAnalysis>,
    optimizations: Vec<CodegenOptimization>,
    proofs: Vec<CodegenProof>,
    proof_attempts: Vec<CodegenProofAttempt>,
    branch_inversion_candidates: Vec<BranchInversionCandidate>,
    deferred_output_cursor: u16,
    suppress_implicit_rts_once: bool,
    inline_byte_constant_shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoutineParameterCapture {
    slot: StorageSlot,
    byte_index: u16,
    store_start: usize,
    store_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallablePointerInfo {
    kind: RoutineKind,
    return_slot: Option<StorageSlot>,
}

#[derive(Debug, Default, Clone)]
struct ModernRoutineLayout {
    for_end_caches: HashMap<SpanKey, ModernForEndCache>,
    string_literals: HashMap<StringLiteralKey, Absolute>,
}

#[derive(Debug, Clone, Copy)]
enum ModernForEndCache {
    Byte(StorageSlot),
    Word { low: StorageSlot, high: StorageSlot },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SpanKey {
    start: usize,
    end: usize,
}

impl From<Span> for SpanKey {
    fn from(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StringLiteralKey {
    span: SpanKey,
    text: String,
}

impl StringLiteralKey {
    fn new(span: Span, text: &str) -> Self {
        Self {
            span: SpanKey::from(span),
            text: text.to_string(),
        }
    }
}

mod opts;
use opts::*;
pub use opts::{CodegenOptimization, CodegenOptimizationKind};

mod expr;

fn routine_has_effect_contract(routine: &Routine) -> bool {
    routine.annotations.iter().any(|annotation| {
        matches!(
            annotation,
            ActioncAnnotation::Preserves { .. }
                | ActioncAnnotation::Clobbers { .. }
                | ActioncAnnotation::Writes { .. }
        )
    })
}

fn routine_uses_debug_compat_profile(routine: &Routine) -> bool {
    routine
        .annotations
        .iter()
        .any(|annotation| matches!(annotation, ActioncAnnotation::DebugProfileCompat))
}

fn codegen_routine_signature_from_ast(
    routine: &Routine,
    params: &[StorageSlot],
    return_slot: Option<StorageSlot>,
) -> CodegenRoutineSignature {
    let params = routine
        .params
        .iter()
        .flat_map(|decl| {
            decl.entries.iter().map(|entry| {
                (
                    entry.name.clone(),
                    type_ref_trace_name(&decl.ty),
                    if decl_is_array_like(decl) {
                        2
                    } else {
                        type_size(&decl.ty).unwrap_or(1)
                    },
                )
            })
        })
        .enumerate()
        .map(
            |(index, (name, type_name, fallback_width))| CodegenRoutineParam {
                name,
                type_name,
                width: params
                    .get(index)
                    .map(|slot| slot.size)
                    .unwrap_or(fallback_width),
            },
        )
        .collect();
    let (kind, return_type, return_width) = match routine.kind {
        RoutineKind::Proc => ("PROC".to_string(), None, None),
        RoutineKind::Func { return_type } => (
            "FUNC".to_string(),
            Some(fund_type_trace_name(return_type).to_string()),
            return_slot.map(|slot| slot.size),
        ),
    };
    CodegenRoutineSignature {
        name: routine.name.clone(),
        kind,
        params,
        return_type,
        return_width,
    }
}

fn type_ref_trace_name(ty: &TypeRef) -> String {
    let mut text = match &ty.base {
        TypeBase::Fund(fund) => fund_type_trace_name(*fund).to_string(),
        TypeBase::Named(name) => name.clone(),
        TypeBase::Callable(kind) => match kind {
            RoutineKind::Proc => "PROC".to_string(),
            RoutineKind::Func { return_type } => {
                format!("{}FUNC", fund_type_trace_name(*return_type))
            }
        },
    };
    if ty.pointer {
        text.push('*');
    }
    text
}

fn fund_type_trace_name(fund: FundType) -> &'static str {
    match fund {
        FundType::Byte => "BYTE",
        FundType::Card => "CARD",
        FundType::Char => "CHAR",
        FundType::Int => "INT",
    }
}

fn routine_trampoline_operand_label(name: &str, byte_index: u16) -> String {
    format!("routine:{}:target:{byte_index}", normalize_name(name))
}

fn collect_numeric_defines(program: &Program) -> HashMap<String, u16> {
    let mut defines = HashMap::new();
    for module in &program.modules {
        for item in &module.items {
            let Item::Define(define) = item else {
                continue;
            };
            for entry in &define.entries {
                if let Some(value) = parse_numeric_define_value(&entry.value) {
                    defines.insert(normalize_name(&entry.name), value);
                }
            }
        }
    }
    defines
}

fn collect_global_callable_pointers(program: &Program) -> HashMap<String, CallablePointerInfo> {
    let mut pointers = HashMap::new();
    for module in &program.modules {
        for item in &module.items {
            let Item::Declaration(Decl::Var(decl)) = item else {
                continue;
            };
            collect_callable_pointer_decl(decl, &mut pointers);
        }
    }
    pointers
}

fn collect_routine_callable_pointers(routine: &Routine) -> HashMap<String, CallablePointerInfo> {
    let mut pointers = HashMap::new();
    for decl in &routine.params {
        collect_callable_pointer_decl(decl, &mut pointers);
    }
    for decl in &routine.locals {
        let Decl::Var(decl) = decl else {
            continue;
        };
        collect_callable_pointer_decl(decl, &mut pointers);
    }
    pointers
}

fn collect_callable_pointer_decl(
    decl: &VarDecl,
    pointers: &mut HashMap<String, CallablePointerInfo>,
) {
    let TypeBase::Callable(kind) = &decl.ty.base else {
        return;
    };
    let return_slot = callable_pointer_return_slot(kind);
    for entry in &decl.entries {
        pointers.insert(
            normalize_name(&entry.name),
            CallablePointerInfo {
                kind: kind.clone(),
                return_slot,
            },
        );
    }
}

fn callable_pointer_return_slot(kind: &RoutineKind) -> Option<StorageSlot> {
    match kind {
        RoutineKind::Proc => None,
        RoutineKind::Func { return_type } => {
            let ty = TypeRef {
                base: TypeBase::Fund(*return_type),
                pointer: false,
            };
            type_size(&ty).map(|size| {
                StorageSlot::zero_page(runtime_zp::ARGS.address(), size).signed(type_is_signed(&ty))
            })
        }
    }
}

fn parse_numeric_define_value(value: &str) -> Option<u16> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Some(hex) = value.strip_prefix('$') {
        return u16::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u16::from_str_radix(hex, 16).ok();
    }
    if let Some(rest) = value.strip_prefix('-') {
        return parse_numeric_define_value(rest).map(|value| 0u16.wrapping_sub(value));
    }
    if let Some(rest) = value.strip_prefix('+') {
        return parse_numeric_define_value(rest);
    }

    value.parse::<u16>().ok()
}

fn builtin_storage_slot(name: &str) -> Option<StorageSlot> {
    let variable = resident_variable(name)?;
    if matches!(variable.kind, ResidentVariableKind::ByteArray { .. }) {
        return Some(StorageSlot::array(
            variable.address,
            1,
            ArrayStorage::Inline,
        ));
    }
    match variable.storage {
        ResidentVariableStorage::Absolute => Some(StorageSlot::absolute(variable.address, 1)),
        ResidentVariableStorage::ZeroPage => {
            Some(StorageSlot::zero_page(variable.address as u8, 1))
        }
    }
}

fn constant_u16(expr: &Expr) -> Option<u16> {
    match &expr.kind {
        ExprKind::Number(number) => number.value,
        ExprKind::Char(ch) => source_char_byte(*ch).map(u16::from),
        ExprKind::Unary {
            op: UnaryOp::Plus,
            expr,
        } => constant_u16(expr),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } => Some(0u16.wrapping_sub(constant_u16(expr)?)),
        ExprKind::Binary { op, left, right } => {
            let left = constant_u16(left)?;
            let right = constant_u16(right)?;
            match op {
                BinaryOp::Add => Some(left.wrapping_add(right)),
                BinaryOp::Sub => Some(left.wrapping_sub(right)),
                BinaryOp::Mul => Some(left.wrapping_mul(right)),
                BinaryOp::Div if right != 0 => Some(left / right),
                BinaryOp::Mod if right != 0 => Some(left % right),
                BinaryOp::Lsh => Some(if right >= 16 {
                    0
                } else {
                    left.wrapping_shl(u32::from(right))
                }),
                BinaryOp::Rsh => Some(if right >= 16 {
                    0
                } else {
                    left.wrapping_shr(u32::from(right))
                }),
                BinaryOp::And => Some(left & right),
                BinaryOp::Or => Some(left | right),
                BinaryOp::Xor => Some(left ^ right),
                _ => None,
            }
        }
        _ => None,
    }
}

fn expr_uses_compatible_runtime_arithmetic(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinaryOp::Mul,
            ..
        }
    )
}

fn normalize_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Define(define) => define
            .entries
            .first()
            .map(|entry| entry.span)
            .unwrap_or_else(|| Span::new(0, 0)),
        Stmt::Return(Some(expr)) => expr.span,
        Stmt::Return(None) => Span::new(0, 0),
        Stmt::Exit { span }
        | Stmt::Assign { span, .. }
        | Stmt::CompoundAssign { span, .. }
        | Stmt::Call { span, .. }
        | Stmt::MachineBlock { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::DoUntil { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Unsupported { span, .. } => *span,
    }
}

fn stmt_source_range_kind(stmt: &Stmt) -> CodegenSourceRangeKind {
    match stmt {
        Stmt::MachineBlock { .. } => CodegenSourceRangeKind::MachineBlock,
        _ => CodegenSourceRangeKind::Statement,
    }
}

fn stmt_source_range_name(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Define(_) => "define",
        Stmt::Return(_) => "return",
        Stmt::Exit { .. } => "exit",
        Stmt::Assign { .. } => "assignment",
        Stmt::CompoundAssign { .. } => "compound assignment",
        Stmt::Call { .. } => "call",
        Stmt::MachineBlock { .. } => "machine block",
        Stmt::If { .. } => "if",
        Stmt::While { .. } => "while",
        Stmt::DoUntil { .. } => "do-until",
        Stmt::For { .. } => "for",
        Stmt::Unsupported { .. } => "unsupported",
    }
}

fn var_decl_source_name(decl: &VarDecl) -> String {
    decl.entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests;
