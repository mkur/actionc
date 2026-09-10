//! Four-byte integer legalization for the classic generator.
//! Types come from the existing SemIR projection and shared scalar contract.
//! Every intermediate has its own computation width before consumer conversion.
use super::*;

const RESULT: StorageSlot = StorageSlot::zero_page(0xC4, 4);
const LEFT: StorageSlot = StorageSlot::zero_page(0x82, 4);
const RIGHT: StorageSlot = StorageSlot::zero_page(0xC0, 4);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WideHelper {
    Mul,
    Div,
    UDiv,
    Mod,
    UMod,
    Lsh,
    Rsh,
}

impl WideHelper {
    pub(super) fn label(self) -> String {
        format!("ACTION.RUNTIME.ACTIONC::{self:?}32")
    }
    pub(super) fn body(self) -> (Vec<u8>, Option<usize>) {
        use crate::integer6502::wide;
        match self {
            Self::Mul => (wide::multiply(), None),
            Self::Lsh | Self::Rsh => (wide::shift_body(self == Self::Lsh), None),
            _ => {
                let body = wide::division(
                    matches!(self, Self::Div | Self::Mod),
                    matches!(self, Self::Mod | Self::UMod),
                );
                (body.bytes, Some(body.error_operand))
            }
        }
    }
}

impl Generator {
    pub(super) fn expr_uses_wide_integer(&self, expr: &Expr) -> bool {
        let Some(ty) = self.expr_scalar_type(expr) else {
            return false;
        };
        if ty.width_bytes() == 4 {
            return true;
        }
        match &expr.kind {
            ExprKind::Cast { expr, .. }
            | ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Neg,
                expr,
            } => self.expr_uses_wide_integer(expr),
            ExprKind::Binary { left, right, .. } => {
                self.expr_uses_wide_integer(left) || self.expr_uses_wide_integer(right)
            }
            ExprKind::Call { args, .. } => args.iter().any(|arg| self.expr_uses_wide_integer(arg)),
            ExprKind::Prepared { value, .. } => self.expr_uses_wide_integer(value),
            _ => false,
        }
    }

    pub(super) fn emit_wide_expr_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        if !self.emit_integer_value(expr) {
            return false;
        }
        for byte in 0..slot.size.min(4) {
            self.emit_lda_slot_byte(RESULT, byte);
            self.emit_sta_slot_byte(slot, byte);
        }
        true
    }

    // A normalized value always occupies C4..C7. Its extra bytes reflect its
    // own signedness, not the signedness/width of an enclosing destination.
    fn normalize_integer_result(&mut self, ty: ScalarType) {
        let width = ty.width_bytes();
        if width == 4 {
            return;
        }
        if ty.signedness() == crate::semantic::ScalarSignedness::Signed {
            let nonnegative = self.next_label("integer-nonnegative");
            self.emit_lda_slot_byte(RESULT, width - 1);
            self.emit_cmp_imm(0x80);
            self.emit_lda_imm(0);
            self.emitter
                .emit_branch_label(opcode::BCC_REL, &nonnegative, Span::new(0, 0));
            self.emit_lda_imm(0xFF);
            self.bind_codegen_label(nonnegative, Span::new(0, 0));
        } else {
            self.emit_lda_imm(0);
        }
        for byte in width..4 {
            self.emit_sta_slot_byte(RESULT, byte);
        }
    }

    fn push_integer_result(&mut self) {
        for byte in 0..4 {
            self.emit_lda_slot_byte(RESULT, byte);
            self.emitter.emit_pha();
        }
    }

    fn pop_integer_left(&mut self) {
        for byte in (0..4).rev() {
            self.emit_pla();
            self.emit_sta_slot_byte(LEFT, byte);
        }
    }

    fn capture_integer_slot(&mut self, slot: StorageSlot) {
        // Capture the complete source before overwriting any result byte.
        for byte in 0..slot.size {
            self.emit_lda_slot_byte(slot, byte);
            self.emitter.emit_pha();
        }
        for byte in (0..slot.size).rev() {
            self.emit_pla();
            self.emit_sta_slot_byte(RESULT, byte);
        }
    }

    pub(super) fn emit_integer_value(&mut self, expr: &Expr) -> bool {
        if let ExprKind::Prepared { statements, value } = &expr.kind {
            self.generate_stmt_list(statements);
            return self.emit_integer_value(value);
        }
        let Some(ty) = self.expr_scalar_type(expr) else {
            return false;
        };
        if !self.expr_uses_wide_integer(expr) {
            if !self.emit_expr_to_slot(
                expr,
                RESULT
                    .with_size(ty.width_bytes())
                    .signed(self.expr_signed(expr)),
            ) {
                return false;
            }
            self.normalize_integer_result(ty);
            return true;
        }
        match &expr.kind {
            ExprKind::Number(number) => {
                let Some(value) = number.value else {
                    return false;
                };
                for (byte, value) in (value as u32).to_le_bytes().into_iter().enumerate() {
                    self.emit_lda_imm(value);
                    self.emit_sta_slot_byte(RESULT, byte as u16);
                }
            }
            ExprKind::Cast { expr, .. } => {
                if !self.emit_integer_value(expr) {
                    return false;
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Neg,
                expr: value,
            } => {
                if !self.emit_integer_value(value) {
                    return false;
                }
                if matches!(
                    expr.kind,
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        ..
                    }
                ) {
                    self.emit_sec();
                    for byte in 0..4 {
                        self.emit_lda_imm(0);
                        self.emit_sbc_slot_byte(RESULT, byte);
                        self.emit_sta_slot_byte(RESULT, byte);
                    }
                }
            }
            ExprKind::Binary { op, left, right } => {
                let compare = branch::CompareOp::from_binary(*op).is_some();
                let operation_ty = if compare {
                    let (Some(left_ty), Some(right_ty)) =
                        (self.expr_scalar_type(left), self.expr_scalar_type(right))
                    else {
                        return false;
                    };
                    ScalarType::promote_binary(left_ty, right_ty)
                } else {
                    ty
                };
                if !self.emit_integer_value(left) {
                    return false;
                }
                self.normalize_integer_result(operation_ty);
                self.push_integer_result();
                if !self.emit_integer_value(right) {
                    return false;
                }
                // A shift count retains all its bits, including high bytes.
                if !matches!(op, BinaryOp::Lsh | BinaryOp::Rsh) {
                    self.normalize_integer_result(operation_ty);
                }
                for byte in 0..4 {
                    self.emit_lda_slot_byte(RESULT, byte);
                    self.emit_sta_slot_byte(RIGHT, byte);
                }
                self.pop_integer_left();
                if !self.emit_integer_binary(*op, operation_ty, expr.span) {
                    return false;
                }
            }
            ExprKind::Call { callee, args }
                if self.array_call_slot_size(callee, args).is_none() =>
            {
                let Some(result) = self.call_return_slot(callee) else {
                    return false;
                };
                if !self.emit_call(callee, args, expr.span) {
                    return false;
                }
                self.capture_integer_slot(result);
            }
            _ => {
                let Some(slot) = self.lvalue_slot(expr) else {
                    return false;
                };
                if !matches!(slot.size, 1 | 2 | 4) {
                    return false;
                }
                self.capture_integer_slot(slot);
            }
        }
        self.normalize_integer_result(ty);
        true
    }

    fn emit_integer_binary(&mut self, op: BinaryOp, ty: ScalarType, span: Span) -> bool {
        let signed = ty.signedness() == crate::semantic::ScalarSignedness::Signed;
        if branch::CompareOp::from_binary(op).is_some() {
            self.emit_integer_comparison(op, signed, span);
            return true;
        }
        let helper = match op {
            BinaryOp::Mul => Some(WideHelper::Mul),
            BinaryOp::Div => Some(if signed {
                WideHelper::Div
            } else {
                WideHelper::UDiv
            }),
            BinaryOp::Mod => Some(if signed {
                WideHelper::Mod
            } else {
                WideHelper::UMod
            }),
            BinaryOp::Lsh => Some(WideHelper::Lsh),
            BinaryOp::Rsh => Some(WideHelper::Rsh),
            _ => None,
        };
        if let Some(helper) = helper {
            self.used_wide_helpers.insert(helper);
            if matches!(
                helper,
                WideHelper::Div | WideHelper::UDiv | WideHelper::Mod | WideHelper::UMod
            ) {
                self.uses_runtime_fault = true;
            }
            self.emitter.emit_jsr_label(helper.label(), span);
            self.processor.invalidate_after_call();
            self.straight_line_store_y = None;
            return true;
        }
        match op {
            BinaryOp::Add => self.emit_clc(),
            BinaryOp::Sub => self.emit_sec(),
            _ => {}
        }
        for byte in 0..4 {
            self.emit_lda_slot_byte(LEFT, byte);
            match op {
                BinaryOp::Add => self.emit_adc_slot_byte(RIGHT, byte),
                BinaryOp::Sub => self.emit_sbc_slot_byte(RIGHT, byte),
                BinaryOp::And => self.emit_and_slot_byte(RIGHT, byte),
                BinaryOp::Or => self.emit_ora_slot_byte(RIGHT, byte),
                BinaryOp::Xor => self.emit_eor_slot_byte(RIGHT, byte),
                _ => return false,
            }
            self.emit_sta_slot_byte(RESULT, byte);
        }
        true
    }

    fn emit_integer_comparison(&mut self, op: BinaryOp, signed: bool, span: Span) {
        let less = self.next_label("integer-less");
        let greater = self.next_label("integer-greater");
        let done = self.next_label("integer-compared");
        if signed {
            self.emit_lda_slot_byte(RIGHT, 3);
            self.emit_eor_imm(0x80);
            self.emit_sta_slot_byte(RIGHT, 3);
        }
        for byte in (0..4).rev() {
            self.emit_lda_slot_byte(LEFT, byte);
            if signed && byte == 3 {
                self.emit_eor_imm(0x80);
            }
            self.emit_cmp_slot_byte(RIGHT, byte);
            self.emitter.emit_branch_label(opcode::BCC_REL, &less, span);
            self.emitter
                .emit_branch_label(opcode::BNE_REL, &greater, span);
        }
        self.emit_lda_imm(u8::from(matches!(
            op,
            BinaryOp::Eq | BinaryOp::Le | BinaryOp::Ge
        )));
        self.emit_jmp_label(done.clone(), span);
        self.bind_codegen_label(less, span);
        self.emit_lda_imm(u8::from(matches!(
            op,
            BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le
        )));
        self.emit_jmp_label(done.clone(), span);
        self.bind_codegen_label(greater, span);
        self.emit_lda_imm(u8::from(matches!(
            op,
            BinaryOp::Ne | BinaryOp::Gt | BinaryOp::Ge
        )));
        self.bind_codegen_label(done, span);
        self.emit_sta_slot_byte(RESULT, 0);
    }

    pub(super) fn emit_wide_assignment(&mut self, target: &Expr, value: &Expr) -> bool {
        let Some(slot) = self.lvalue_slot(target) else {
            return false;
        };
        if !matches!(slot.size, 1 | 2 | 4) {
            return false;
        }
        let pointer = runtime_zp::ARRAY_ADDR;
        if !self.emit_slot_address(slot, pointer) {
            return false;
        }
        self.emit_lda_zero_page_value_only(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page_value_only(pointer);
        self.emitter.emit_pha();
        if !self.emit_integer_value(value) {
            return false;
        }
        self.emit_pla();
        self.emit_sta_zero_page(pointer);
        self.emit_pla();
        self.emit_sta_zero_page(pointer.offset(1));
        let destination =
            StorageSlot::indirect_indexed_y(pointer, slot.size).volatile(slot.is_volatile);
        for byte in 0..slot.size {
            self.emit_lda_slot_byte(RESULT, byte);
            self.emit_sta_slot_byte(destination, byte);
        }
        true
    }

    pub(super) fn emit_wide_compound_assignment(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
        span: Span,
    ) -> bool {
        let Some(target_ty) = self.expr_scalar_type(target) else {
            return false;
        };
        let Some(operation_ty) = self
            .compound_operations
            .operation(self.current_record_copy_scope.as_deref(), span)
            .and_then(|operation| operation.result_type.as_scalar())
        else {
            return false;
        };
        let Some(slot) = self.lvalue_slot(target) else {
            return false;
        };
        let pointer = runtime_zp::ARRAY_ADDR;
        if !self.emit_slot_address(slot, pointer) {
            return false;
        }
        self.emit_lda_zero_page_value_only(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page_value_only(pointer);
        self.emitter.emit_pha();
        if !self.emit_integer_value(value) {
            return false;
        }
        if !matches!(op, BinaryOp::Lsh | BinaryOp::Rsh) {
            self.normalize_integer_result(operation_ty);
        }
        for byte in 0..4 {
            self.emit_lda_slot_byte(RESULT, byte);
            self.emit_sta_slot_byte(RIGHT, byte);
        }
        self.emit_pla();
        self.emit_sta_zero_page(pointer);
        self.emit_pla();
        self.emit_sta_zero_page(pointer.offset(1));
        self.emit_lda_zero_page_value_only(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page_value_only(pointer);
        self.emitter.emit_pha();
        // Capture the address before RHS effects, but read its value afterwards.
        let destination = StorageSlot::indirect_indexed_y(pointer, slot.size)
            .signed(slot.signed)
            .volatile(slot.is_volatile);
        self.capture_integer_slot(destination);
        self.normalize_integer_result(target_ty);
        self.normalize_integer_result(operation_ty);
        for byte in 0..4 {
            self.emit_lda_slot_byte(RESULT, byte);
            self.emit_sta_slot_byte(LEFT, byte);
        }
        if !self.emit_integer_binary(op, operation_ty, span) {
            return false;
        }
        self.normalize_integer_result(operation_ty);
        self.emit_pla();
        self.emit_sta_zero_page(pointer);
        self.emit_pla();
        self.emit_sta_zero_page(pointer.offset(1));
        for byte in 0..slot.size {
            self.emit_lda_slot_byte(RESULT, byte);
            self.emit_sta_slot_byte(destination, byte);
        }
        true
    }

    pub(super) fn generate_wide_for(
        &mut self,
        target: &Expr,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        span: Span,
    ) {
        // SemIR projects its resolved direction and magnitude into these literals.
        let (down, magnitude) = match step.map(|step| &step.kind) {
            None => (false, Some(1)),
            Some(ExprKind::Number(number)) => (false, number.value),
            Some(ExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            }) => (
                true,
                if let ExprKind::Number(number) = &expr.kind {
                    number.value
                } else {
                    None
                },
            ),
            _ => (false, None),
        };
        let Some(amount) = magnitude.filter(|amount| (1..=u32::MAX as u64).contains(amount)) else {
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen only supports constant non-zero FOR steps",
            ));
            return;
        };
        let ty = self.expr_scalar_type(target).unwrap();
        let signed = ty.signedness() == crate::semantic::ScalarSignedness::Signed;
        let constant = |bits| Expr {
            kind: ExprKind::Number(crate::semantic::ConstValue { ty, bits }.number_literal()),
            text: String::new(),
            span,
        };
        self.generate_assignment(target, start, span);
        let test = self.next_label("long-for:test");
        let done = self.next_label("long-for:done");
        self.bind_codegen_label(test.clone(), span);
        let limit = if down { BinaryOp::Ge } else { BinaryOp::Le };
        if !self.emit_branch_if_false_compare(limit, target, end, &done, span) {
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen could not compare LONG FOR bounds",
            ));
            return;
        }
        self.exit_labels.push(done.clone());
        self.generate_stmt_list(body);
        self.exit_labels.pop();
        let maximum = if signed { 0x7FFF_FFFFu64 } else { 0xFFFF_FFFF };
        let minimum = if signed { 0x8000_0000u64 } else { 0 };
        if amount <= if signed { 0x8000_0000 } else { maximum } {
            let (op, bits) = if down {
                (BinaryOp::Lt, minimum.wrapping_add(amount) & 0xFFFF_FFFF)
            } else {
                (BinaryOp::Gt, maximum.wrapping_sub(amount) & 0xFFFF_FFFF)
            };
            self.emit_branch_if_true_compare(op, target, &constant(bits), &done, span);
        }
        let incremented = Expr {
            kind: ExprKind::Binary {
                op: if down { BinaryOp::Sub } else { BinaryOp::Add },
                left: Box::new(target.clone()),
                right: Box::new(constant(amount)),
            },
            text: String::new(),
            span,
        };
        self.generate_assignment(target, &incremented, span);
        self.emit_jmp_label(test, span);
        self.bind_codegen_label(done, span);
    }

    pub(super) fn emit_wide_branch(&mut self, expr: &Expr, label: &str, span: Span) -> bool {
        if !self.emit_integer_value(expr) {
            return false;
        }
        self.emit_lda_slot_byte(RESULT, 0);
        for byte in 1..4 {
            self.emit_ora_slot_byte(RESULT, byte);
        }
        let next = self.next_label("integer-zero");
        self.emitter.emit_branch_label(opcode::BEQ_REL, &next, span);
        self.emit_jmp_label(label.to_owned(), span);
        self.bind_codegen_label(next, span);
        true
    }
}
