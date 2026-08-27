use super::*;

const REAL_BYTES: u16 = 6;
const REAL_TEMP_PREFIX: &str = "$ACTIONC_NATIVE_REAL_TEMP_";
const REAL_INTEGER_TEMP: &str = "$ACTIONC_NATIVE_REAL_INTEGER";
const REAL_SIGN_TEMP: &str = "$ACTIONC_NATIVE_REAL_SIGN";
const REAL_ADDRESS_TEMP: &str = "$ACTIONC_NATIVE_REAL_ADDRESS";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ClassicAtariFppService {
    IntegerToFloat,
    FloatToInteger,
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl ClassicAtariFppService {
    pub(super) const fn address(self) -> u16 {
        match self {
            Self::IntegerToFloat => 0xD9AA,
            Self::FloatToInteger => 0xD9D2,
            Self::Subtract => 0xDA60,
            Self::Add => 0xDA66,
            Self::Multiply => 0xDADB,
            Self::Divide => 0xDB28,
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::IntegerToFloat => "IFP",
            Self::FloatToInteger => "FPI",
            Self::Add => "FADD",
            Self::Subtract => "FSUB",
            Self::Multiply => "FMULT",
            Self::Divide => "FDIV",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ClassicNativeRealFacts {
    expressions: HashMap<(String, usize, usize), ClassicNativeExpr>,
}

#[derive(Debug, Clone)]
pub(super) enum ClassicNativeExpr {
    Real(ClassicRealValue),
    Compare {
        op: BinaryOp,
        left: ClassicRealValue,
        right: ClassicRealValue,
    },
    ToInteger {
        value: ClassicRealValue,
    },
}

#[derive(Debug, Clone)]
pub(super) enum ClassicRealValue {
    Literal([u8; REAL_BYTES as usize]),
    Place(Expr),
    IntegerToReal {
        source: Expr,
        width: u16,
        signed: bool,
    },
    Unary {
        op: UnaryOp,
        value: Box<ClassicRealValue>,
    },
    Binary {
        op: BinaryOp,
        left: Box<ClassicRealValue>,
        right: Box<ClassicRealValue>,
    },
}

impl ClassicNativeRealFacts {
    pub(super) fn extend(&mut self, other: Self) {
        self.expressions.extend(other.expressions);
    }

    pub(super) fn insert(
        &mut self,
        scope: Option<&str>,
        span: Span,
        expression: ClassicNativeExpr,
    ) {
        self.expressions.insert(
            (scope.unwrap_or_default().to_owned(), span.start, span.end),
            expression,
        );
    }

    pub(super) fn expression(&self, scope: Option<&str>, span: Span) -> Option<&ClassicNativeExpr> {
        self.expressions
            .get(&(scope.unwrap_or_default().to_owned(), span.start, span.end))
    }

    pub(super) fn remove(&mut self, scope: Option<&str>, span: Span) {
        self.expressions
            .remove(&(scope.unwrap_or_default().to_owned(), span.start, span.end));
    }
}

fn native_real_literal_label(bytes: [u8; REAL_BYTES as usize]) -> String {
    let suffix = bytes
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("native-real:literal:{suffix}")
}

fn is_direct_real_slot(slot: StorageSlot) -> bool {
    matches!(slot.space, AddressSpace::Absolute | AddressSpace::ZeroPage)
}

fn real_y_index_start(slot: StorageSlot) -> Option<u8> {
    let start = if slot.space == AddressSpace::IndirectIndexedY {
        slot.index_offset
    } else if is_direct_real_slot(slot) {
        0
    } else {
        return None;
    };
    start
        .checked_add(REAL_BYTES.saturating_sub(1))
        .filter(|end| *end <= u16::from(u8::MAX))?;
    Some(start as u8)
}

pub(super) fn real_temp_name(index: usize) -> String {
    format!("{REAL_TEMP_PREFIX}{index}")
}

pub(super) fn real_integer_temp_name() -> &'static str {
    REAL_INTEGER_TEMP
}

pub(super) fn real_sign_temp_name() -> &'static str {
    REAL_SIGN_TEMP
}

pub(super) fn real_address_temp_name() -> &'static str {
    REAL_ADDRESS_TEMP
}

pub(super) fn is_native_real_hidden_name(name: &str) -> bool {
    name.starts_with(REAL_TEMP_PREFIX)
        || matches!(name, REAL_INTEGER_TEMP | REAL_SIGN_TEMP | REAL_ADDRESS_TEMP)
}

#[derive(Debug, Clone, Copy)]
enum PreparedRealTarget {
    Direct(StorageSlot),
    SavedIndirect,
}

impl Generator {
    pub(super) fn try_emit_native_real_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> Option<bool> {
        if self.native_real_fact_suppression > 0 {
            return None;
        }
        let ClassicNativeExpr::Real(value) = self
            .native_real
            .expression(self.current_native_real_scope.as_deref(), value.span)?
            .clone()
        else {
            return None;
        };
        Some(self.emit_native_real_assignment(target, &value))
    }

    pub(super) fn try_emit_native_real_compound_assignment(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
    ) -> Option<bool> {
        if self.native_real_fact_suppression > 0 {
            return None;
        }
        let ClassicNativeExpr::Real(left) = self
            .native_real
            .expression(self.current_native_real_scope.as_deref(), target.span)?
            .clone()
        else {
            return None;
        };
        let ClassicNativeExpr::Real(right) = self
            .native_real
            .expression(self.current_native_real_scope.as_deref(), value.span)?
            .clone()
        else {
            return None;
        };
        let value = ClassicRealValue::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
        Some(self.emit_native_real_assignment(target, &value))
    }

    pub(super) fn try_emit_native_real_expr_to_slot(
        &mut self,
        expr: &Expr,
        target: StorageSlot,
    ) -> Option<bool> {
        if self.native_real_fact_suppression > 0 {
            return None;
        }
        let expression = self
            .native_real
            .expression(self.current_native_real_scope.as_deref(), expr.span)?
            .clone();
        Some(match expression {
            ClassicNativeExpr::Real(value) => {
                self.emit_real_value_to_slot(&value, target, 0, expr.span)
            }
            ClassicNativeExpr::Compare { op, left, right } => {
                self.emit_real_compare_to_slot(op, &left, &right, target, expr.span)
            }
            ClassicNativeExpr::ToInteger { value } => {
                self.emit_real_to_integer(&value, target, expr.span)
            }
        })
    }

    pub(super) fn try_emit_native_real_branch_if_true(
        &mut self,
        condition: &Expr,
        target: &str,
        span: Span,
    ) -> Option<bool> {
        if self.native_real_fact_suppression > 0 {
            return None;
        }
        let expression = self
            .native_real
            .expression(self.current_native_real_scope.as_deref(), condition.span)?
            .clone();
        Some(match expression {
            ClassicNativeExpr::Compare { op, left, right } => {
                self.emit_real_compare_branch(op, &left, &right, target, span)
            }
            ClassicNativeExpr::Real(value) => {
                let Some(value_temp) = self.real_temp(0) else {
                    return Some(false);
                };
                if !self.emit_real_value_to_slot(&value, value_temp, 1, span) {
                    return Some(false);
                }
                for offset in 0..REAL_BYTES {
                    self.emit_lda_slot_byte(value_temp, offset);
                    self.emitter
                        .emit_branch_label(opcode::BNE_REL, target, span);
                }
                true
            }
            ClassicNativeExpr::ToInteger { .. } => return None,
        })
    }

    fn emit_native_real_assignment(&mut self, target: &Expr, value: &ClassicRealValue) -> bool {
        let span = target.span;
        let Some(target) = self.prepare_real_assignment_target(target) else {
            return false;
        };
        let Some(staged) = self.real_temp(0) else {
            return false;
        };
        if !self.emit_real_value_to_slot(value, staged, 1, span) {
            return false;
        }
        let Some(target) = self.restore_real_assignment_target(target) else {
            return false;
        };
        self.emit_staged_real_copy(staged, target)
    }

    fn prepare_real_assignment_target(&mut self, target: &Expr) -> Option<PreparedRealTarget> {
        self.native_real_fact_suppression += 1;
        let slot = self.lvalue_slot(target);
        self.native_real_fact_suppression -= 1;
        let slot = slot?;
        if slot.size != REAL_BYTES || slot.array.is_some() || slot.pointee_size.is_some() {
            return None;
        }
        match slot.space {
            AddressSpace::Absolute | AddressSpace::ZeroPage => {
                Some(PreparedRealTarget::Direct(slot))
            }
            AddressSpace::IndirectIndexedY => {
                let address = self.lookup_slot(real_address_temp_name())?;
                let offset = slot.index_offset;
                self.emit_clc();
                self.emit_lda_zero_page(slot.zero_page_byte(0));
                self.emit_adc_immediate(Immediate::new(offset), 0);
                self.emit_sta_slot_byte(address, 0);
                self.emit_lda_zero_page(slot.zero_page_byte(0).offset(1));
                self.emit_adc_immediate(Immediate::new(offset), 1);
                self.emit_sta_slot_byte(address, 1);
                Some(PreparedRealTarget::SavedIndirect)
            }
            AddressSpace::AbsoluteX => None,
        }
    }

    fn restore_real_assignment_target(
        &mut self,
        target: PreparedRealTarget,
    ) -> Option<StorageSlot> {
        match target {
            PreparedRealTarget::Direct(slot) => Some(slot),
            PreparedRealTarget::SavedIndirect => {
                let address = self.lookup_slot(real_address_temp_name())?;
                self.emit_slot_byte_to_zero_page(address, 0, runtime_zp::ARRAY_ADDR);
                self.emit_slot_byte_to_zero_page(address, 1, runtime_zp::ARRAY_ADDR.offset(1));
                Some(StorageSlot::indirect_indexed_y(
                    runtime_zp::ARRAY_ADDR,
                    REAL_BYTES,
                ))
            }
        }
    }

    fn emit_real_value_to_slot(
        &mut self,
        value: &ClassicRealValue,
        target: StorageSlot,
        depth: usize,
        span: Span,
    ) -> bool {
        if target.size != REAL_BYTES || target.array.is_some() || target.pointee_size.is_some() {
            return false;
        }
        match value {
            ClassicRealValue::Literal(bytes) => self.emit_real_constant(*bytes, target, span),
            ClassicRealValue::Place(source) => {
                self.native_real_fact_suppression += 1;
                let source_slot = self.lvalue_slot(source);
                self.native_real_fact_suppression -= 1;
                let Some(source) = source_slot else {
                    return false;
                };
                source.size == REAL_BYTES && self.emit_staged_real_copy(source, target)
            }
            ClassicRealValue::IntegerToReal {
                source,
                width,
                signed,
            } => {
                if let Some(bytes) = self.constant_integer_real_bytes(source, *width, *signed) {
                    self.emit_real_constant(bytes, target, span)
                } else {
                    self.emit_integer_to_real(source, *width, *signed, target, span)
                }
            }
            ClassicRealValue::Unary { op, value } => match op {
                UnaryOp::Plus => self.emit_real_value_to_slot(value, target, depth, span),
                UnaryOp::Neg => {
                    let Some(operand) = self.real_temp(depth) else {
                        return false;
                    };
                    if !self.emit_real_value_to_slot(value, operand, depth + 1, span) {
                        return false;
                    }
                    self.emit_negated_real_copy(operand, target, span)
                }
                UnaryOp::Deref | UnaryOp::AddressOf => false,
            },
            ClassicRealValue::Binary { op, left, right } => {
                let Some(left_temp) = self.real_temp(depth) else {
                    return false;
                };
                let Some(right_temp) = self.real_temp(depth + 1) else {
                    return false;
                };
                if !self.emit_real_value_to_slot(left, left_temp, depth + 2, span)
                    || !self.emit_real_value_to_slot(right, right_temp, depth + 2, span)
                {
                    return false;
                }
                let service = match op {
                    BinaryOp::Add => ClassicAtariFppService::Add,
                    BinaryOp::Sub => ClassicAtariFppService::Subtract,
                    BinaryOp::Mul => ClassicAtariFppService::Multiply,
                    BinaryOp::Div => ClassicAtariFppService::Divide,
                    _ => return false,
                };
                let fr0 = StorageSlot::zero_page(0xD4, REAL_BYTES);
                let fr1 = StorageSlot::zero_page(0xE0, REAL_BYTES);
                if !self.emit_staged_real_copy(left_temp, fr0)
                    || !self.emit_staged_real_copy(right_temp, fr1)
                {
                    return false;
                }
                self.emit_atari_fpp_call(service);
                self.emit_staged_real_copy(fr0, target)
            }
        }
    }

    fn constant_integer_real_bytes(
        &self,
        source: &Expr,
        width: u16,
        signed: bool,
    ) -> Option<[u8; REAL_BYTES as usize]> {
        let value = self.constant_u16(source)?;
        let decimal = match (width, signed) {
            (1, false) => (value as u8).to_string(),
            (1, true) => (value as u8 as i8).to_string(),
            (2, false) => value.to_string(),
            (2, true) => (value as i16).to_string(),
            _ => return None,
        };
        crate::atari_real::AtariReal::from_decimal(&decimal)
            .ok()
            .map(|value| value.to_bytes())
    }

    fn emit_real_constant(
        &mut self,
        bytes: [u8; REAL_BYTES as usize],
        target: StorageSlot,
        span: Span,
    ) -> bool {
        let label = if let Some((label, _)) = self.native_real_literal_pool.get(&bytes) {
            label.clone()
        } else {
            let label = native_real_literal_label(bytes);
            self.native_real_literal_pool
                .insert(bytes, (label.clone(), span));
            label
        };
        self.emit_pooled_real_copy(&label, target, span)
    }

    fn emit_integer_to_real(
        &mut self,
        source: &Expr,
        width: u16,
        signed: bool,
        target: StorageSlot,
        span: Span,
    ) -> bool {
        if !matches!(width, 1 | 2) {
            return false;
        }
        let Some(integer) = self.lookup_slot(real_integer_temp_name()) else {
            return false;
        };
        let Some(sign) = self.lookup_slot(real_sign_temp_name()) else {
            return false;
        };
        if !self.emit_expr_to_slot_without_native_real(source, integer) {
            return false;
        }
        if width == 1 {
            if signed {
                let nonnegative = self.next_label("real:ifp-nonnegative-byte");
                let extended = self.next_label("real:ifp-extended-byte");
                self.emit_lda_slot_byte(integer, 0);
                self.emitter
                    .emit_branch_label(opcode::BPL_REL, &nonnegative, span);
                self.emit_lda_imm(0xFF);
                self.emit_jmp_label(&extended, span);
                self.bind_codegen_label(nonnegative, span);
                self.emit_lda_imm(0);
                self.bind_codegen_label(extended, span);
                self.emit_sta_slot_byte(integer, 1);
            } else {
                self.emit_lda_imm(0);
                self.emit_sta_slot_byte(integer, 1);
            }
        }
        if signed {
            self.emit_lda_slot_byte(integer, 1);
            self.emit_and_imm(0x80);
            self.emit_sta_slot_byte(sign, 0);
            let magnitude_ready = self.next_label("real:ifp-magnitude-ready");
            self.emitter
                .emit_branch_label(opcode::BEQ_REL, &magnitude_ready, span);
            self.emit_sec();
            self.emit_lda_imm(0);
            self.emit_sbc_slot_byte(integer, 0);
            self.emit_sta_slot_byte(integer, 0);
            self.emit_lda_imm(0);
            self.emit_sbc_slot_byte(integer, 1);
            self.emit_sta_slot_byte(integer, 1);
            self.bind_codegen_label(magnitude_ready, span);
        } else {
            self.emit_lda_imm(0);
            self.emit_sta_slot_byte(sign, 0);
        }

        let fr0 = StorageSlot::zero_page(0xD4, REAL_BYTES);
        self.emit_copy_slot_byte_to_slot_byte(integer, 0, fr0, 0);
        self.emit_copy_slot_byte_to_slot_byte(integer, 1, fr0, 1);
        self.emit_atari_fpp_call(ClassicAtariFppService::IntegerToFloat);
        self.emit_lda_slot_byte(fr0, 0);
        self.emit_ora_slot_byte(sign, 0);
        self.emit_sta_slot_byte(fr0, 0);
        self.emit_staged_real_copy(fr0, target)
    }

    fn emit_real_to_integer(
        &mut self,
        value: &ClassicRealValue,
        target: StorageSlot,
        span: Span,
    ) -> bool {
        if !matches!(target.size, 1 | 2) {
            return false;
        }
        let Some(source) = self.real_temp(0) else {
            return false;
        };
        let Some(integer) = self.lookup_slot(real_integer_temp_name()) else {
            return false;
        };
        let Some(sign) = self.lookup_slot(real_sign_temp_name()) else {
            return false;
        };
        if !self.emit_real_value_to_slot(value, source, 1, span) {
            return false;
        }
        let fr0 = StorageSlot::zero_page(0xD4, REAL_BYTES);
        if !self.emit_staged_real_copy(source, fr0) {
            return false;
        }
        self.emit_lda_slot_byte(fr0, 0);
        self.emit_and_imm(0x80);
        self.emit_sta_slot_byte(sign, 0);
        self.emit_lda_slot_byte(fr0, 0);
        self.emit_and_imm(0x7F);
        self.emit_sta_slot_byte(fr0, 0);
        self.emit_atari_fpp_call(ClassicAtariFppService::FloatToInteger);
        self.emit_copy_slot_byte_to_slot_byte(fr0, 0, integer, 0);
        self.emit_copy_slot_byte_to_slot_byte(fr0, 1, integer, 1);
        let magnitude_ready = self.next_label("real:fpi-magnitude-ready");
        self.emit_lda_slot_byte(sign, 0);
        self.emitter
            .emit_branch_label(opcode::BEQ_REL, &magnitude_ready, span);
        self.emit_sec();
        self.emit_lda_imm(0);
        self.emit_sbc_slot_byte(integer, 0);
        self.emit_sta_slot_byte(integer, 0);
        self.emit_lda_imm(0);
        self.emit_sbc_slot_byte(integer, 1);
        self.emit_sta_slot_byte(integer, 1);
        self.bind_codegen_label(magnitude_ready, span);
        self.emit_copy_slot_to_slot(integer.with_size(target.size), target)
    }

    fn emit_real_compare_to_slot(
        &mut self,
        op: BinaryOp,
        left: &ClassicRealValue,
        right: &ClassicRealValue,
        target: StorageSlot,
        span: Span,
    ) -> bool {
        let true_label = self.next_label("real:compare-true");
        let done_label = self.next_label("real:compare-done");
        self.emit_store_constant(target, 0);
        if !self.emit_real_compare_branch(op, left, right, &true_label, span) {
            return false;
        }
        self.emit_jmp_label(&done_label, span);
        self.bind_codegen_label(true_label, span);
        self.emit_store_constant(target, 1);
        self.bind_codegen_label(done_label, span);
        true
    }

    fn emit_real_compare_branch(
        &mut self,
        op: BinaryOp,
        left: &ClassicRealValue,
        right: &ClassicRealValue,
        target: &str,
        span: Span,
    ) -> bool {
        let Some(left_temp) = self.real_temp(0) else {
            return false;
        };
        let Some(right_temp) = self.real_temp(1) else {
            return false;
        };
        if !self.emit_real_value_to_slot(left, left_temp, 2, span)
            || !self.emit_real_value_to_slot(right, right_temp, 2, span)
        {
            return false;
        }
        self.emit_real_relation_branch(op, left_temp, right_temp, target, span)
    }

    fn emit_real_relation_branch(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: StorageSlot,
        target: &str,
        span: Span,
    ) -> bool {
        if !matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) {
            return false;
        }
        let same_sign = self.next_label("real:compare-same-sign");
        let negative = self.next_label("real:compare-negative");
        let less = self.next_label("real:compare-less");
        let equal = self.next_label("real:compare-equal");
        let greater = self.next_label("real:compare-greater");
        let done = self.next_label("real:compare-false");

        self.emit_lda_slot_byte(left, 0);
        self.emit_eor_slot_byte(right, 0);
        self.emitter
            .emit_branch_label(opcode::BPL_REL, &same_sign, span);
        self.emit_lda_slot_byte(left, 0);
        let sign_greater = self.next_label("real:compare-sign-greater");
        self.emitter
            .emit_branch_label(opcode::BPL_REL, &sign_greater, span);
        self.emit_jmp_label(&less, span);
        self.bind_codegen_label(sign_greater, span);
        self.emit_jmp_label(&greater, span);

        self.bind_codegen_label(same_sign, span);
        self.emit_lda_slot_byte(left, 0);
        self.emitter
            .emit_branch_label(opcode::BMI_REL, &negative, span);
        self.emit_real_lexicographic_dispatch(left, right, &less, &equal, &greater, span);

        self.bind_codegen_label(negative, span);
        self.emit_real_lexicographic_dispatch(left, right, &greater, &equal, &less, span);

        for (label, relation) in [(less, -1i8), (equal, 0i8), (greater, 1i8)] {
            self.bind_codegen_label(label, span);
            let accepted = match op {
                BinaryOp::Eq => relation == 0,
                BinaryOp::Ne => relation != 0,
                BinaryOp::Lt => relation < 0,
                BinaryOp::Le => relation <= 0,
                BinaryOp::Gt => relation > 0,
                BinaryOp::Ge => relation >= 0,
                _ => false,
            };
            self.emit_jmp_label(if accepted { target } else { &done }, span);
        }
        self.bind_codegen_label(done, span);
        true
    }

    fn emit_real_lexicographic_dispatch(
        &mut self,
        left: StorageSlot,
        right: StorageSlot,
        less: &str,
        equal: &str,
        greater: &str,
        span: Span,
    ) {
        let local_less = self.next_label("real:lex-less");
        let local_greater = self.next_label("real:lex-greater");
        for offset in 0..REAL_BYTES {
            self.emit_lda_slot_byte(left, offset);
            self.emit_cmp_slot_byte(right, offset);
            self.emitter
                .emit_branch_label(opcode::BCC_REL, &local_less, span);
            self.emitter
                .emit_branch_label(opcode::BNE_REL, &local_greater, span);
        }
        self.emit_jmp_label(equal, span);
        self.bind_codegen_label(local_less, span);
        self.emit_jmp_label(less, span);
        self.bind_codegen_label(local_greater, span);
        self.emit_jmp_label(greater, span);
    }

    fn emit_staged_real_copy(&mut self, source: StorageSlot, target: StorageSlot) -> bool {
        if source.size != REAL_BYTES || target.size != REAL_BYTES {
            return false;
        }
        if source == target {
            return true;
        }
        if source.space == AddressSpace::IndirectIndexedY
            && target.space == AddressSpace::IndirectIndexedY
        {
            return false;
        }

        if source.is_volatile || target.is_volatile {
            return self.emit_unrolled_staged_real_copy(source, target);
        }

        if is_direct_real_slot(source) && is_direct_real_slot(target) {
            let source_start = source.address;
            let target_start = target.address;
            let requires_forward_copy = target_start < source_start
                && target_start.saturating_add(REAL_BYTES) > source_start;
            if requires_forward_copy {
                for offset in 0..REAL_BYTES {
                    self.emit_copy_slot_byte_to_slot_byte(source, offset, target, offset);
                }
                return true;
            }
            return self.emit_compact_direct_real_copy(source, target);
        }

        self.emit_compact_staged_real_copy(source, target)
    }

    fn emit_unrolled_staged_real_copy(&mut self, source: StorageSlot, target: StorageSlot) -> bool {
        for offset in 0..REAL_BYTES {
            self.emit_lda_slot_byte_value_only(source, offset);
            self.emitter.emit_pha();
        }
        for offset in (0..REAL_BYTES).rev() {
            self.emit_pla();
            self.emit_sta_slot_byte(target, offset);
        }
        true
    }

    fn emit_compact_direct_real_copy(&mut self, source: StorageSlot, target: StorageSlot) -> bool {
        let span = Span::new(0, 0);
        let loop_label = self.next_label("real:copy");
        self.emit_ldx_imm((REAL_BYTES - 1) as u8);
        self.bind_codegen_label(loop_label.clone(), span);
        self.emit_real_indexed_x_load(source);
        self.emit_real_indexed_x_store(target);
        self.emitter.emit_dex();
        self.emitter
            .emit_branch_label(opcode::BPL_REL, loop_label, span);
        self.finish_compact_real_copy(target, true, false);
        true
    }

    fn emit_compact_staged_real_copy(&mut self, source: StorageSlot, target: StorageSlot) -> bool {
        let Some(source_index) = real_y_index_start(source) else {
            return false;
        };
        let Some(target_index) = real_y_index_start(target) else {
            return false;
        };
        let span = Span::new(0, 0);
        let load_loop = self.next_label("real:stack-copy-load");
        let store_loop = self.next_label("real:stack-copy-store");

        self.emit_ldy_imm(source_index);
        self.bind_codegen_label(load_loop.clone(), span);
        self.emit_real_indexed_y_load(source);
        self.emitter.emit_pha();
        self.emit_iny();
        self.emitter
            .emit_cpy_imm(source_index.wrapping_add(REAL_BYTES as u8));
        self.emitter
            .emit_branch_label(opcode::BNE_REL, load_loop, span);

        self.emit_ldy_imm(target_index.wrapping_add((REAL_BYTES - 1) as u8));
        self.bind_codegen_label(store_loop.clone(), span);
        self.emit_pla();
        self.emit_real_indexed_y_store(target);
        self.emit_dey();
        if target_index == 0 {
            self.emitter
                .emit_branch_label(opcode::BPL_REL, store_loop, span);
        } else {
            self.emitter.emit_cpy_imm(target_index.wrapping_sub(1));
            self.emitter
                .emit_branch_label(opcode::BNE_REL, store_loop, span);
        }
        self.finish_compact_real_copy(target, false, true);
        true
    }

    fn emit_pooled_real_copy(&mut self, label: &str, target: StorageSlot, span: Span) -> bool {
        if target.size != REAL_BYTES {
            return false;
        }
        if is_direct_real_slot(target) {
            let loop_label = self.next_label("real:literal-copy");
            self.emit_ldx_imm((REAL_BYTES - 1) as u8);
            self.bind_codegen_label(loop_label.clone(), span);
            self.emitter.emit_u8(opcode::LDA_ABS_X);
            self.emitter.emit_u16_label(label, span);
            self.emit_real_indexed_x_store(target);
            self.emitter.emit_dex();
            self.emitter
                .emit_branch_label(opcode::BPL_REL, loop_label, span);
            self.finish_compact_real_copy(target, true, false);
            return true;
        }
        if target.space != AddressSpace::IndirectIndexedY {
            return false;
        }

        let Some(target_index) = real_y_index_start(target) else {
            return false;
        };
        let load_loop = self.next_label("real:literal-stack-load");
        let store_loop = self.next_label("real:literal-stack-store");
        self.emit_ldy_imm(0);
        self.bind_codegen_label(load_loop.clone(), span);
        self.emitter.emit_u8(opcode::LDA_ABS_Y);
        self.emitter.emit_u16_label(label, span);
        self.emitter.emit_pha();
        self.emit_iny();
        self.emitter.emit_cpy_imm(REAL_BYTES as u8);
        self.emitter
            .emit_branch_label(opcode::BNE_REL, load_loop, span);

        self.emit_ldy_imm(target_index.wrapping_add((REAL_BYTES - 1) as u8));
        self.bind_codegen_label(store_loop.clone(), span);
        self.emit_pla();
        self.emit_real_indexed_y_store(target);
        self.emit_dey();
        if target_index == 0 {
            self.emitter
                .emit_branch_label(opcode::BPL_REL, store_loop, span);
        } else {
            self.emitter.emit_cpy_imm(target_index.wrapping_sub(1));
            self.emitter
                .emit_branch_label(opcode::BNE_REL, store_loop, span);
        }
        self.finish_compact_real_copy(target, false, true);
        true
    }

    fn emit_real_indexed_x_load(&mut self, source: StorageSlot) {
        match source.space {
            AddressSpace::Absolute => self
                .emitter
                .emit_lda_absolute_x(source.absolute_x_operand()),
            AddressSpace::ZeroPage => self
                .emitter
                .emit_lda_zero_page_x(ZeroPageX::new(source.address as u8)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => unreachable!(),
        }
    }

    fn emit_real_indexed_x_store(&mut self, target: StorageSlot) {
        match target.space {
            AddressSpace::Absolute => self
                .emitter
                .emit_sta_absolute_x(target.absolute_x_operand()),
            AddressSpace::ZeroPage => self
                .emitter
                .emit_sta_zero_page_x(ZeroPageX::new(target.address as u8)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => unreachable!(),
        }
    }

    fn emit_real_indexed_y_load(&mut self, source: StorageSlot) {
        match source.space {
            AddressSpace::Absolute | AddressSpace::ZeroPage => {
                self.emitter.emit_lda_absolute_y(source.absolute_byte(0))
            }
            AddressSpace::IndirectIndexedY => self
                .emitter
                .emit_lda_indirect_indexed_y(IndirectIndexedY::new(source.zero_page_byte(0))),
            AddressSpace::AbsoluteX => unreachable!(),
        }
    }

    fn emit_real_indexed_y_store(&mut self, target: StorageSlot) {
        match target.space {
            AddressSpace::Absolute | AddressSpace::ZeroPage => {
                self.emitter.emit_sta_absolute_y(target.absolute_byte(0))
            }
            AddressSpace::IndirectIndexedY => self
                .emitter
                .emit_sta_indirect_indexed_y(IndirectIndexedY::new(target.zero_page_byte(0))),
            AddressSpace::AbsoluteX => unreachable!(),
        }
    }

    fn finish_compact_real_copy(
        &mut self,
        target: StorageSlot,
        clobbers_x: bool,
        clobbers_y: bool,
    ) {
        match target.space {
            AddressSpace::Absolute => {
                self.record_current_absolute_write(target.address, REAL_BYTES);
                self.processor
                    .invalidate_prepared_pointers_touching_range(target.address, REAL_BYTES);
            }
            AddressSpace::ZeroPage => {
                for offset in 0..REAL_BYTES {
                    self.record_current_zero_page_write(target.zero_page_byte(offset));
                }
                self.processor.invalidate_all_zp();
            }
            AddressSpace::IndirectIndexedY => {
                self.record_current_unknown_absolute_write();
                self.processor.invalidate_all_zp();
            }
            AddressSpace::AbsoluteX => unreachable!(),
        }
        self.processor.invalidate_accumulator();
        if clobbers_x {
            self.processor.invalidate_index_x();
        }
        if clobbers_y {
            self.processor.invalidate_index_y();
            self.processor.invalidate_carry();
            self.straight_line_store_y = None;
        }
        self.processor.invalidate_memory();
    }

    pub(super) fn emit_native_real_literal_pool(&mut self) {
        let entries = self
            .native_real_literal_pool
            .iter()
            .map(|(bytes, (label, span))| (*bytes, label.clone(), *span))
            .collect::<Vec<_>>();
        for (bytes, label, span) in entries {
            if let Err(diagnostic) = self.emitter.bind_label(label, span) {
                self.diagnostics.push(diagnostic);
                continue;
            }
            let start = self.current_absolute_address();
            for byte in bytes {
                self.emitter.emit_u8(byte);
            }
            let end = self.current_absolute_address();
            self.record_source_range(
                CodegenSourceRangeKind::StorageInitializer,
                Some("native REAL literal pool".to_string()),
                span,
                start,
                end,
            );
        }
    }

    fn emit_negated_real_copy(
        &mut self,
        source: StorageSlot,
        target: StorageSlot,
        span: Span,
    ) -> bool {
        if !self.emit_staged_real_copy(source, target) {
            return false;
        }

        let toggle = self.next_label("real:neg-toggle");
        let done = self.next_label("real:neg-done");
        self.emit_lda_slot_byte(target, 0);
        self.emit_and_imm(0x7F);
        self.emitter
            .emit_branch_label(opcode::BNE_REL, &toggle, span);
        for offset in 1..REAL_BYTES {
            self.emit_lda_slot_byte(target, offset);
            self.emitter
                .emit_branch_label(opcode::BNE_REL, &toggle, span);
        }

        // A is zero on this path. Normalize either spelling of zero to the
        // canonical positive representation without invoking FPP subtraction.
        self.emit_sta_slot_byte(target, 0);
        self.emit_jmp_label(&done, span);

        self.bind_codegen_label(toggle, span);
        self.emit_lda_slot_byte(target, 0);
        self.emit_eor_imm(0x80);
        self.emit_sta_slot_byte(target, 0);
        self.bind_codegen_label(done, span);
        true
    }

    fn emit_atari_fpp_call(&mut self, service: ClassicAtariFppService) {
        self.used_atari_fpp_services.insert(service);
        self.record_current_unknown_effects();
        self.emitter
            .emit_jsr_absolute(Absolute::new(service.address()));
        // Compatible Atari math packs do not agree on the returned decimal
        // flag. Generated ADC/SBC sequences require binary mode.
        self.emitter.emit_cld();
        self.processor.invalidate_after_call();
        self.straight_line_store_y = None;
    }

    fn real_temp(&self, index: usize) -> Option<StorageSlot> {
        self.lookup_slot(&real_temp_name(index))
            .filter(|slot| slot.size == REAL_BYTES && slot.array.is_none())
    }
}
