use super::array::indexed_expr_index;
use super::proof::{
    IndexAddressMode, ValueAvailabilityProof, ValueAvailabilitySource, ValueByteAvailability,
};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    pub(super) fn from_binary(op: BinaryOp) -> Option<Self> {
        match op {
            BinaryOp::Eq => Some(Self::Eq),
            BinaryOp::Ne => Some(Self::Ne),
            BinaryOp::Lt => Some(Self::Lt),
            BinaryOp::Le => Some(Self::Le),
            BinaryOp::Gt => Some(Self::Gt),
            BinaryOp::Ge => Some(Self::Ge),
            _ => None,
        }
    }

    pub(super) fn to_binary(self) -> BinaryOp {
        match self {
            Self::Eq => BinaryOp::Eq,
            Self::Ne => BinaryOp::Ne,
            Self::Lt => BinaryOp::Lt,
            Self::Le => BinaryOp::Le,
            Self::Gt => BinaryOp::Gt,
            Self::Ge => BinaryOp::Ge,
        }
    }

    pub(super) fn reversed_operands(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Gt => Self::Lt,
            Self::Ge => Self::Le,
            op => op,
        }
    }

    pub(super) fn equality_branch(self) -> Option<u8> {
        match self {
            Self::Eq => Some(opcode::BEQ_REL),
            Self::Ne => Some(opcode::BNE_REL),
            _ => None,
        }
    }

    pub(super) fn unsigned_order_branch_plan(self) -> Option<(bool, u8)> {
        match self {
            Self::Lt => Some((false, opcode::BCC_REL)),
            Self::Le => Some((true, opcode::BCS_REL)),
            Self::Gt => Some((true, opcode::BCC_REL)),
            Self::Ge => Some((false, opcode::BCS_REL)),
            _ => None,
        }
    }

    pub(super) fn signed_order_branch_plan(self) -> Option<(bool, u8)> {
        match self {
            Self::Lt => Some((false, opcode::BMI_REL)),
            Self::Le => Some((true, opcode::BPL_REL)),
            Self::Gt => Some((true, opcode::BMI_REL)),
            Self::Ge => Some((false, opcode::BPL_REL)),
            _ => None,
        }
    }

    pub(super) fn ordered_operand_plan(self) -> Option<(bool, bool)> {
        match self {
            Self::Lt => Some((false, false)),
            Self::Le => Some((false, true)),
            Self::Gt => Some((true, false)),
            Self::Ge => Some((true, true)),
            _ => None,
        }
    }

    pub(super) fn signed_zero_lvalue_branch(self) -> Option<u8> {
        match self {
            Self::Lt => Some(opcode::BMI_REL),
            Self::Ge => Some(opcode::BPL_REL),
            _ => None,
        }
    }

    pub(super) fn strict_order_operand_swap(self) -> Option<bool> {
        match self {
            Self::Lt => Some(false),
            Self::Gt => Some(true),
            _ => None,
        }
    }
}

pub(super) fn reverse_compare_op(op: BinaryOp) -> BinaryOp {
    CompareOp::from_binary(op)
        .map(|op| op.reversed_operands().to_binary())
        .unwrap_or(op)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompareBranchFlags {
    Equality,
    UnsignedOrder,
    SignedOrder,
}

pub(super) fn debug_assert_compare_branch_opcode(opcode: u8, flags: CompareBranchFlags) {
    let allowed = match flags {
        CompareBranchFlags::Equality => matches!(opcode, opcode::BEQ_REL | opcode::BNE_REL),
        CompareBranchFlags::UnsignedOrder => {
            matches!(
                opcode,
                opcode::BCC_REL | opcode::BCS_REL | opcode::BEQ_REL | opcode::BNE_REL
            )
        }
        CompareBranchFlags::SignedOrder => matches!(opcode, opcode::BMI_REL | opcode::BPL_REL),
    };
    debug_assert!(
        allowed,
        "branch opcode ${opcode:02X} does not match compare flag source {flags:?}"
    );
}

pub(super) fn debug_assert_compare_slot_expr_shape(left: StorageSlot, width: u16) {
    debug_assert!(width > 0, "comparison width must be non-zero");
    debug_assert!(
        left.size >= width,
        "left comparison slot must cover every compared byte"
    );
    debug_assert!(
        left.pointee_size.is_none(),
        "comparison slot must be a value slot, not pointer storage"
    );
    debug_assert!(
        left.array.is_none(),
        "comparison slot must be a value slot, not array storage"
    );
}

pub(super) fn debug_assert_compare_slots_shape(left: StorageSlot, right: StorageSlot, width: u16) {
    debug_assert_compare_slot_expr_shape(left, width);
    debug_assert!(
        right.size >= width,
        "right comparison slot must cover every compared byte"
    );
    debug_assert!(
        right.pointee_size.is_none(),
        "right comparison slot must be a value slot, not pointer storage"
    );
    debug_assert!(
        right.array.is_none(),
        "right comparison slot must be a value slot, not array storage"
    );
}

impl Generator {
    pub(super) fn emit_branch_if_false(
        &mut self,
        condition: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        if self.segment_storage
            && !self.profile.enables_modern_optimizations()
            && let ExprKind::Binary { op, left, right } = &condition.kind
            && matches!(op, BinaryOp::And | BinaryOp::Or)
            && (Self::is_condition_shaped_expr(left) || Self::is_condition_shaped_expr(right))
        {
            return self.emit_branch_if_false_logical(*op, left, right, label, span);
        }

        let true_label = self.next_label("condition:true");
        if !self.emit_branch_if_true(condition, &true_label, span) {
            return false;
        }
        let true_processor = self.processor.clone();
        let branch_start = self.emitter.position().saturating_sub(2);
        let true_y_hint = self.processor.y_immediate();
        let false_y_hint = self.processor.y_immediate();
        let true_straight_line_store_y = self.straight_line_store_y;
        self.emit_jmp_label(label, span);
        self.maybe_record_branch_inversion_candidate(branch_start, &true_label, label, span);
        if let Some(y) = false_y_hint {
            self.label_store_y_hints.insert(label.to_string(), y);
        }
        if let Some(y) = true_y_hint {
            self.label_store_y_hints.insert(true_label.clone(), y);
        }
        self.bind_codegen_label_preserving_state(
            true_label,
            span,
            true_processor,
            true_straight_line_store_y,
        );
        true
    }

    pub(super) fn emit_branch_if_false_logical(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        match op {
            BinaryOp::And => {
                if !self.emit_branch_if_false(left, label, span) {
                    return false;
                }
                if !self.emit_branch_if_false(right, label, span) {
                    return false;
                }
                if self.segment_storage && !self.profile.enables_modern_optimizations() {
                    self.processor.invalidate_index_y();
                    self.straight_line_store_y = None;
                }
                true
            }
            BinaryOp::Or => {
                let true_label = self.next_label("condition:true");
                if !self.emit_branch_if_true(left, &true_label, span) {
                    return false;
                }
                if !self.emit_branch_if_false(right, label, span) {
                    return false;
                }
                self.bind_codegen_label(true_label, span);
                true
            }
            _ => false,
        }
    }

    pub(super) fn emit_branch_if_true(
        &mut self,
        condition: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        match &condition.kind {
            ExprKind::Binary { op, left, right }
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                ) =>
            {
                self.emit_branch_if_true_compare(*op, left, right, label, span)
            }
            ExprKind::Binary { op, left, right }
                if self.segment_storage
                    && matches!(op, BinaryOp::And | BinaryOp::Or)
                    && (Self::is_condition_shaped_expr(left)
                        || Self::is_condition_shaped_expr(right)) =>
            {
                self.emit_branch_if_true_logical(*op, left, right, label, span)
            }
            ExprKind::Binary { op, left, right }
                if self.segment_storage
                    && matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) =>
            {
                self.emit_branch_if_true_bitwise(*op, left, right, label, span)
            }
            ExprKind::Call { callee, args }
                if self.array_call_slot_size(callee, args).is_none() =>
            {
                self.emit_branch_if_true_call_return(callee, args, label, span)
            }
            _ => self.emit_branch_if_true_nonzero(condition, label, span),
        }
    }

    pub(super) fn is_condition_shaped_expr(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                ) || (matches!(op, BinaryOp::And | BinaryOp::Or)
                    && (Self::is_condition_shaped_expr(left)
                        || Self::is_condition_shaped_expr(right)))
            }
            _ => false,
        }
    }

    pub(super) fn emit_branch_if_true_logical(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        match op {
            BinaryOp::And => {
                let false_label = self.next_label("condition:false");
                if !self.emit_branch_if_false(left, &false_label, span) {
                    return false;
                }
                if !self.emit_branch_if_true(right, label, span) {
                    return false;
                }
                self.bind_codegen_label(false_label, span);
                true
            }
            BinaryOp::Or => {
                if !self.emit_branch_if_true(left, label, span) {
                    return false;
                }
                self.emit_branch_if_true(right, label, span)
            }
            _ => false,
        }
    }

    pub(super) fn emit_branch_if_true_nonzero(
        &mut self,
        condition: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let width = self.expr_size(condition).unwrap_or(1);
        for byte_index in 0..width {
            if !self.emit_load_simple_byte(condition, byte_index) {
                return false;
            }
            self.emitter.emit_branch_label(opcode::BNE_REL, label, span);
        }
        true
    }

    pub(super) fn emit_branch_if_true_call_return(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        label: &str,
        span: Span,
    ) -> bool {
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };
        if !self.emit_call(callee, args, span) {
            return false;
        }
        for byte_index in 0..return_slot.size {
            self.emit_lda_slot_byte(return_slot, byte_index);
            self.emitter.emit_branch_label(opcode::BNE_REL, label, span);
        }
        true
    }

    pub(super) fn emit_branch_if_true_bitwise(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let width = self
            .expr_size(left)
            .unwrap_or(1)
            .max(self.expr_size(right).unwrap_or(1));
        if self.profile.enables_modern_optimizations()
            && width <= 2
            && expr_contains_routine_call(left, &self.routines)
            && expr_contains_routine_call(right, &self.routines)
        {
            let left_slot = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), width)
                .signed(self.expr_signed(left));
            if !self.emit_expr_to_slot(left, left_slot) {
                return false;
            }
            let right_slot = StorageSlot::zero_page(runtime_zp::ADDR.address(), width)
                .signed(self.expr_signed(right));
            if !self.emit_expr_to_slot(right, right_slot) {
                return false;
            }
            for byte_index in 0..width {
                if !self.emit_binary_slot_slot_byte(op, left_slot, right_slot, byte_index, false) {
                    return false;
                }
                if self.profile.enables_modern_optimizations() {
                    self.emitter.emit_branch_label(opcode::BNE_REL, label, span);
                    continue;
                }
                let temp = runtime_zp::ARRAY_ADDR.offset(byte_index as u8);
                self.emit_sta_zero_page(temp);
                self.emit_lda_zero_page(temp);
                self.emitter.emit_branch_label(opcode::BNE_REL, label, span);
            }
            return true;
        }
        for byte_index in 0..width {
            if !self.emit_binary_expr_byte(op, left, right, byte_index, false) {
                return false;
            }
            if self.profile.enables_modern_optimizations() {
                self.emitter.emit_branch_label(opcode::BNE_REL, label, span);
                continue;
            }
            let temp = runtime_zp::ARRAY_ADDR.offset(byte_index as u8);
            self.emit_sta_zero_page(temp);
            self.emit_lda_zero_page(temp);
            self.emitter.emit_branch_label(opcode::BNE_REL, label, span);
        }
        true
    }
}

impl Generator {
    pub(super) fn emit_branch_if_false_compare(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let true_label = self.next_label("compare:true");
        if !self.emit_branch_if_true_compare(op, left, right, &true_label, span) {
            return false;
        }
        let true_processor = self.processor.clone();
        let true_straight_line_store_y = self.straight_line_store_y;
        let branch_start = self.emitter.position().saturating_sub(2);
        self.emit_jmp_label(label, span);
        self.maybe_record_branch_inversion_candidate(branch_start, &true_label, label, span);
        self.bind_codegen_label_preserving_state(
            true_label,
            span,
            true_processor,
            true_straight_line_store_y,
        );
        true
    }

    pub(super) fn emit_branch_if_true_compare(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let width = self
            .expr_size(left)
            .unwrap_or(1)
            .max(self.expr_size(right).unwrap_or(1));
        if self.segment_storage
            && width == 1
            && matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            )
            && let Some(value) = self.constant_u16(right)
            && self.emit_call_return_constant_branch(left, value, op, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 1
            && matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            )
            && let Some(value) = self.constant_u16(left)
            && self.emit_call_return_constant_branch(
                right,
                value,
                reverse_compare_op(op),
                label,
                span,
            )
        {
            return true;
        }
        if self.segment_storage
            && width == 2
            && self.expr_signed(left)
            && matches!(
                op,
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
            )
            && let Some(value) = self.constant_u16(right)
            && self.emit_call_return_constant_branch(left, value, op, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 1
            && self.is_pointer_deref_expr(left)
            && self.is_pointer_deref_expr(right)
            && self.emit_pointer_deref_compare_branch(op, left, right, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 1
            && matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            )
            && self.is_index_expr(left)
            && self.is_index_expr(right)
            && self.emit_compatible_indexed_byte_compare_branch(op, left, right, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 2
            && matches!(op, BinaryOp::Lt | BinaryOp::Gt)
            && self.is_index_expr(left)
            && self.is_index_expr(right)
            && self.emit_compatible_indexed_word_ordered_branch(op, left, right, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 1
            && matches!(op, BinaryOp::Eq | BinaryOp::Ne)
            && self.is_index_expr(left)
            && !Self::compare_operand_needs_materialization(right)
        {
            return match op {
                BinaryOp::Eq => self.emit_byte_eq_branch(left, right, label, span),
                BinaryOp::Ne => self.emit_byte_ne_branch(left, right, label, span),
                _ => false,
            };
        }
        if self.segment_storage
            && width == 1
            && matches!(op, BinaryOp::Eq | BinaryOp::Ne)
            && self.is_index_expr(right)
            && !Self::compare_operand_needs_materialization(left)
        {
            return match op {
                BinaryOp::Eq => self.emit_byte_eq_branch(right, left, label, span),
                BinaryOp::Ne => self.emit_byte_ne_branch(right, left, label, span),
                _ => false,
            };
        }
        if self.segment_storage
            && width == 1
            && matches!(
                op,
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
            )
            && ((self.is_index_expr(left) && !Self::compare_operand_needs_materialization(right))
                || (self.is_index_expr(right)
                    && !Self::compare_operand_needs_materialization(left)))
        {
            return self.emit_compatible_byte_ordered_branch(op, left, right, label, span);
        }
        if width == 2 && self.emit_modern_signed_zero_branch(op, left, right, label, span) {
            return true;
        }
        if self.segment_storage
            && width == 1
            && self.emit_compatible_bitwise_zero_compare_branch(op, left, right, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 2
            && self.emit_signed_zero_lvalue_branch(op, left, right, label, span)
        {
            return true;
        }
        if self.emit_compatible_materialized_call_compare_with_stack(
            op, left, right, width, label, span,
        ) {
            return true;
        }
        if self.segment_storage && width <= 2 && Self::compare_operand_needs_materialization(left) {
            let slot = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), width)
                .signed(self.expr_signed(left));
            if !self.emit_expr_to_slot(left, slot) {
                return false;
            }
            return self.emit_branch_if_true_compare_slot_expr(op, slot, right, width, label, span);
        }
        if self.segment_storage && width <= 2 && Self::compare_operand_needs_materialization(right)
        {
            let left_slot = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), width)
                .signed(self.expr_signed(left));
            if !self.emit_expr_to_slot(left, left_slot) {
                return false;
            }
            let right_slot = StorageSlot::zero_page(runtime_zp::ADDR.address(), width)
                .signed(self.expr_signed(right));
            if !self.emit_expr_to_slot(right, right_slot) {
                return false;
            }
            return self
                .emit_branch_if_true_compare_slots(op, left_slot, right_slot, width, label, span);
        }
        match op {
            BinaryOp::Eq if self.segment_storage && width == 1 => {
                self.emit_byte_eq_branch(left, right, label, span)
            }
            BinaryOp::Ne if self.segment_storage && width == 1 => {
                self.emit_byte_ne_branch(left, right, label, span)
            }
            BinaryOp::Eq if self.segment_storage && width == 2 => {
                self.emit_word_eq_branch(left, right, label, span)
            }
            BinaryOp::Ne if self.segment_storage && width == 2 => {
                self.emit_word_ne_branch(left, right, label, span)
            }
            BinaryOp::Eq => self.emit_eq_branch(left, right, width, label, span),
            BinaryOp::Ne => self.emit_ne_branch(left, right, width, label, span),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
                if self.segment_storage && self.is_signed_compare(left, right, width) =>
            {
                self.emit_compatible_signed_ordered_branch(op, left, right, label, span)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
                if self.segment_storage && width == 1 =>
            {
                self.emit_compatible_byte_ordered_branch(op, left, right, label, span)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
                if self.segment_storage && width == 2 =>
            {
                self.emit_compatible_unsigned_ordered_branch(op, left, right, label, span)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
                if self.is_signed_compare(left, right, width) =>
            {
                self.emit_signed_ordered_branch(op, left, right, label, span)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                self.emit_ordered_branch(op, left, right, width, label, span)
            }
            _ => false,
        }
    }

    pub(super) fn emit_compatible_bitwise_zero_compare_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(compare_op) = CompareOp::from_binary(op) else {
            return false;
        };
        let Some(branch) = compare_op.equality_branch() else {
            return false;
        };
        let expr = if self.constant_u16(right) == Some(0) {
            left
        } else if self.constant_u16(left) == Some(0) {
            right
        } else {
            return false;
        };
        let ExprKind::Binary {
            op: bitwise_op,
            left: bitwise_left,
            right: bitwise_right,
        } = &expr.kind
        else {
            return false;
        };
        if !matches!(bitwise_op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
            return false;
        }

        if !self.emit_binary_expr_byte(*bitwise_op, bitwise_left, bitwise_right, 0, false) {
            return false;
        }
        let temp = runtime_zp::ARRAY_ADDR;
        self.emit_sta_zero_page(temp);
        self.emit_lda_zero_page(temp);
        self.emit_compare_branch_label(branch, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_pointer_deref_compare_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(op) = CompareOp::from_binary(op) else {
            return false;
        };
        match op {
            CompareOp::Eq | CompareOp::Ne => {
                let Some(right_slot) =
                    self.pointer_deref_slot_with_pointer_expr(right, runtime_zp::ELEMENT_ADDR)
                else {
                    return false;
                };
                let Some(left_slot) =
                    self.pointer_deref_slot_with_pointer_expr(left, runtime_zp::ARRAY_ADDR)
                else {
                    return false;
                };
                self.emit_lda_slot_byte(left_slot, 0);
                self.emit_eor_slot_byte(right_slot, 0);
                let branch = op.equality_branch().expect("equality branch");
                self.emit_compare_branch_label(branch, CompareBranchFlags::Equality, label, span);
                true
            }
            CompareOp::Lt => self.emit_pointer_deref_ordered_branch(left, right, true, label, span),
            CompareOp::Le => {
                self.emit_pointer_deref_ordered_branch(left, right, false, label, span)
            }
            CompareOp::Gt => self.emit_pointer_deref_ordered_branch(right, left, true, label, span),
            CompareOp::Ge => {
                self.emit_pointer_deref_ordered_branch(right, left, false, label, span)
            }
        }
    }

    pub(super) fn emit_pointer_deref_ordered_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        strict: bool,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(right_slot) =
            self.pointer_deref_slot_with_pointer_expr(right, runtime_zp::ELEMENT_ADDR)
        else {
            return false;
        };
        let Some(left_slot) =
            self.pointer_deref_slot_with_pointer_expr(left, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        self.emit_lda_slot_byte(left_slot, 0);
        self.emit_cmp_slot_byte(right_slot, 0);
        self.emit_compare_branch_label(
            opcode::BCC_REL,
            CompareBranchFlags::UnsignedOrder,
            label,
            span,
        );
        if !strict {
            self.emit_compare_branch_label(
                opcode::BEQ_REL,
                CompareBranchFlags::UnsignedOrder,
                label,
                span,
            );
        }
        true
    }

    pub(super) fn emit_compatible_indexed_byte_compare_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(op) = CompareOp::from_binary(op) else {
            return false;
        };
        let Some(left_slot) = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        let Some(right_slot) =
            self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ELEMENT_ADDR)
        else {
            return false;
        };
        if left_slot.size != 1
            || right_slot.size != 1
            || left_slot.space != AddressSpace::IndirectIndexedY
            || right_slot.space != AddressSpace::IndirectIndexedY
        {
            return false;
        }

        if let Some(branch) = op.equality_branch() {
            self.emit_lda_slot_byte(left_slot, 0);
            self.emit_cmp_slot_byte(right_slot, 0);
            self.emit_compare_branch_label(branch, CompareBranchFlags::Equality, label, span);
            return true;
        }
        let Some((swap, branch)) = op.unsigned_order_branch_plan() else {
            return false;
        };
        let (first, second) = if swap {
            (right_slot, left_slot)
        } else {
            (left_slot, right_slot)
        };
        self.emit_lda_slot_byte(first, 0);
        self.emit_cmp_slot_byte(second, 0);
        self.emit_compare_branch_label(branch, CompareBranchFlags::UnsignedOrder, label, span);
        true
    }

    pub(super) fn emit_compatible_indexed_word_ordered_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(op) = CompareOp::from_binary(op) else {
            return false;
        };
        let Some(left_slot) = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        let Some(right_slot) =
            self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ELEMENT_ADDR)
        else {
            return false;
        };
        if left_slot.size != 2
            || right_slot.size != 2
            || left_slot.space != AddressSpace::IndirectIndexedY
            || right_slot.space != AddressSpace::IndirectIndexedY
        {
            return false;
        }

        let Some(swap) = op.strict_order_operand_swap() else {
            return false;
        };
        let (first, second) = if swap {
            (right_slot, left_slot)
        } else {
            (left_slot, right_slot)
        };
        self.emit_lda_slot_byte(first, 0);
        self.emit_cmp_slot_byte(second, 0);
        self.emit_lda_slot_byte(first, 1);
        self.emit_sbc_slot_byte(second, 1);
        self.preserve_y_one_for_branch_target(label);
        let (branch, flags) = if self.is_signed_compare(left, right, 2) {
            (opcode::BMI_REL, CompareBranchFlags::SignedOrder)
        } else {
            (opcode::BCC_REL, CompareBranchFlags::UnsignedOrder)
        };
        self.emit_compare_branch_label(branch, flags, label, span);
        true
    }

    pub(super) fn emit_call_return_constant_branch(
        &mut self,
        expr: &Expr,
        value: u16,
        op: BinaryOp,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(compare_op) = CompareOp::from_binary(op) else {
            return false;
        };
        let ExprKind::Call { callee, args } = &expr.kind else {
            return false;
        };
        if self.array_call_slot_size(callee, args).is_some() {
            return false;
        }
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };
        let proof = self.value_availability_proof(expr);
        if !self.emit_call(callee, args, span) {
            return false;
        }
        if value == 0
            && self.emit_modern_signed_zero_slot_branch(compare_op, return_slot, label, span)
        {
            return true;
        }
        let immediate = Immediate::new(value);
        match (return_slot.size, return_slot.signed, compare_op) {
            (1, _, CompareOp::Eq | CompareOp::Ne) => {
                if !self.profile.enables_modern_optimizations()
                    || !self.emit_proven_value_byte_from_proof(&proof, 0, span)
                {
                    self.emit_lda_slot_byte(return_slot, 0);
                }
                if value != 0 {
                    self.emit_eor_immediate(immediate, 0);
                }
                let branch = compare_op.equality_branch().expect("equality branch");
                self.emit_compare_branch_label(branch, CompareBranchFlags::Equality, label, span);
                true
            }
            (1, _, CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge) => {
                let Some((swap, branch)) = compare_op.unsigned_order_branch_plan() else {
                    return false;
                };
                if swap {
                    self.emit_lda_immediate(immediate, 0);
                    self.emit_cmp_slot_byte(return_slot, 0);
                } else {
                    if !self.profile.enables_modern_optimizations()
                        || !self.emit_proven_value_byte_from_proof(&proof, 0, span)
                    {
                        self.emit_lda_slot_byte(return_slot, 0);
                    }
                    self.emit_cmp_immediate(immediate, 0);
                }
                self.emit_compare_branch_label(
                    branch,
                    CompareBranchFlags::UnsignedOrder,
                    label,
                    span,
                );
                true
            }
            (2, true, CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge) => {
                let Some((swap, branch)) = compare_op.signed_order_branch_plan() else {
                    return false;
                };
                if swap {
                    self.emit_lda_immediate(immediate, 0);
                    self.emit_cmp_slot_byte(return_slot, 0);
                    self.emit_lda_immediate(immediate, 1);
                    self.emit_sbc_slot_byte(return_slot, 1);
                } else {
                    self.emit_lda_slot_byte(return_slot, 0);
                    self.emit_cmp_immediate(immediate, 0);
                    self.emit_lda_slot_byte(return_slot, 1);
                    self.emit_sbc_immediate(immediate, 1);
                }
                self.emit_compare_branch_label(
                    branch,
                    CompareBranchFlags::SignedOrder,
                    label,
                    span,
                );
                true
            }
            _ => false,
        }
    }

    pub(super) fn is_pointer_deref_expr(&self, expr: &Expr) -> bool {
        matches!(
            expr.kind,
            ExprKind::Unary {
                op: UnaryOp::Deref,
                ..
            }
        )
    }

    pub(super) fn is_index_expr(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Index { .. } => true,
            ExprKind::Call { callee, args } => self.array_call_slot_size(callee, args).is_some(),
            _ => false,
        }
    }

    fn emit_compatible_materialized_call_compare_with_stack(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        if !self.segment_storage
            || self.profile.enables_modern_optimizations()
            || width > 2
            || !Self::compare_operand_needs_materialization(left)
            || !Self::compare_operand_needs_materialization(right)
            || !expr_contains_routine_call(right, &self.routines)
        {
            return false;
        }

        let left_slot = StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), width)
            .signed(self.expr_signed(left));
        if !self.emit_expr_to_slot(left, left_slot) {
            return false;
        }
        for byte_index in (0..width).rev() {
            self.emit_lda_slot_byte(left_slot, byte_index);
            self.emitter.emit_pha();
        }

        let right_slot = StorageSlot::zero_page(runtime_zp::ARGS.address(), width)
            .signed(self.expr_signed(right));
        if !self.emit_expr_to_slot(right, right_slot) {
            return false;
        }

        for byte_index in 0..width {
            self.emit_pla();
            self.emit_sta_slot_byte(left_slot, byte_index);
        }
        self.emit_branch_if_true_compare_slots(op, left_slot, right_slot, width, label, span)
    }

    pub(super) fn compare_operand_needs_materialization(expr: &Expr) -> bool {
        matches!(
            expr.kind,
            ExprKind::Call { .. }
                | ExprKind::Unary {
                    op: UnaryOp::Neg,
                    ..
                }
                | ExprKind::Binary {
                    op: BinaryOp::Add
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
                }
        )
    }

    pub(super) fn compare_right_operand_needs_materialization(expr: &Expr) -> bool {
        Self::compare_operand_needs_materialization(expr)
    }

    pub(super) fn emit_branch_if_true_compare_slot_expr(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(compare_op) = CompareOp::from_binary(op) else {
            return false;
        };
        debug_assert_compare_slot_expr_shape(left, width);
        if Self::compare_right_operand_needs_materialization(right) {
            let right_slot = StorageSlot::zero_page(runtime_zp::ADDR.address(), width);
            if !self.emit_expr_to_slot(right, right_slot) {
                return false;
            }
            return self
                .emit_branch_if_true_compare_slots(op, left, right_slot, width, label, span);
        }
        if self.constant_u16(right) == Some(0)
            && self.emit_modern_signed_zero_slot_branch(compare_op, left, label, span)
        {
            return true;
        }
        if self.segment_storage
            && width == 2
            && matches!(op, BinaryOp::Eq | BinaryOp::Ne)
            && !left.signed
            && let Some(right_slot) = self.direct_scalar_slot(right)
            && !right_slot.signed
        {
            return self.emit_compatible_word_slot_equality_branch(
                left, right_slot, compare_op, label, span,
            );
        }

        match compare_op {
            CompareOp::Lt
                if self.segment_storage
                    && width == 2
                    && left.signed
                    && self.constant_u16(right) == Some(0) =>
            {
                self.emit_lda_slot_byte(left, 1);
                self.emit_compare_branch_label(
                    opcode::BMI_REL,
                    CompareBranchFlags::SignedOrder,
                    label,
                    span,
                );
                true
            }
            CompareOp::Gt
                if self.segment_storage
                    && width == 2
                    && left.signed
                    && self.constant_u16(right) == Some(0) =>
            {
                let done_label = self.next_label("compare:done");
                self.emit_lda_slot_byte(left, 1);
                self.emit_compare_branch_label(
                    opcode::BMI_REL,
                    CompareBranchFlags::SignedOrder,
                    &done_label,
                    span,
                );
                self.emit_compare_branch_label(
                    opcode::BNE_REL,
                    CompareBranchFlags::Equality,
                    label,
                    span,
                );
                self.emit_lda_slot_byte(left, 0);
                self.emit_compare_branch_label(
                    opcode::BNE_REL,
                    CompareBranchFlags::Equality,
                    label,
                    span,
                );
                self.bind_codegen_label(done_label, span);
                true
            }
            CompareOp::Eq => {
                let done_label = self.next_label("compare:done");
                for byte_index in (0..width).rev() {
                    if !self.emit_compare_slot_expr_byte(left, right, byte_index) {
                        return false;
                    }
                    self.emit_equality_branch_step(compare_op, label, &done_label, span);
                }
                self.emit_jmp_label(label, span);
                self.bind_codegen_label(done_label, span);
                true
            }
            CompareOp::Ne => {
                for byte_index in (0..width).rev() {
                    if !self.emit_compare_slot_expr_byte(left, right, byte_index) {
                        return false;
                    }
                    self.emit_equality_branch_step(compare_op, label, label, span);
                }
                true
            }
            CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge => {
                self.emit_ordered_branch_slot_expr(compare_op, left, right, width, label, span)
            }
        }
    }

    pub(super) fn emit_branch_if_true_compare_slots(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: StorageSlot,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(compare_op) = CompareOp::from_binary(op) else {
            return false;
        };
        debug_assert_compare_slots_shape(left, right, width);
        if self.segment_storage
            && width == 2
            && (left.signed || right.signed)
            && compare_op.signed_order_branch_plan().is_some()
        {
            return self
                .emit_compatible_signed_ordered_branch_slots(compare_op, left, right, label, span);
        }

        match compare_op {
            CompareOp::Eq | CompareOp::Ne
                if self.segment_storage && width == 2 && !left.signed && !right.signed =>
            {
                self.emit_compatible_word_slot_equality_branch(left, right, compare_op, label, span)
            }
            CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge
                if self.segment_storage && width == 2 && !left.signed && !right.signed =>
            {
                self.emit_compatible_unsigned_ordered_branch_slots(
                    compare_op, left, right, label, span,
                )
            }
            CompareOp::Eq => {
                let done_label = self.next_label("compare:done");
                for byte_index in (0..width).rev() {
                    self.emit_compare_slot_slot_byte(left, right, byte_index);
                    self.emit_equality_branch_step(compare_op, label, &done_label, span);
                }
                self.emit_jmp_label(label, span);
                self.bind_codegen_label(done_label, span);
                true
            }
            CompareOp::Ne => {
                for byte_index in (0..width).rev() {
                    self.emit_compare_slot_slot_byte(left, right, byte_index);
                    self.emit_equality_branch_step(compare_op, label, label, span);
                }
                true
            }
            CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge => {
                self.emit_ordered_branch_slots(compare_op, left, right, width, label, span)
            }
        }
    }

    pub(super) fn emit_compatible_unsigned_ordered_branch_slots(
        &mut self,
        op: CompareOp,
        left: StorageSlot,
        right: StorageSlot,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap, branch)) = op.unsigned_order_branch_plan() else {
            return false;
        };
        let (first, second) = if swap { (right, left) } else { (left, right) };
        self.emit_lda_slot_byte(first, 0);
        self.emit_cmp_slot_byte(second, 0);
        self.emit_lda_slot_byte(first, 1);
        self.emit_sbc_slot_byte(second, 1);
        self.preserve_y_one_for_branch_target(label);
        self.emit_compare_branch_label(branch, CompareBranchFlags::UnsignedOrder, label, span);
        true
    }

    pub(super) fn emit_compatible_word_slot_equality_branch(
        &mut self,
        left: StorageSlot,
        right: StorageSlot,
        op: CompareOp,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        self.emit_lda_slot_byte(left, 0);
        self.emit_eor_slot_byte(right, 0);
        self.emit_compare_branch_label(
            opcode::BNE_REL,
            CompareBranchFlags::Equality,
            &done_label,
            span,
        );
        self.emit_ora_slot_byte(left, 1);
        self.emit_eor_slot_byte(right, 1);
        self.bind_codegen_label(done_label, span);
        let Some(branch) = op.equality_branch() else {
            return false;
        };
        self.emit_compare_branch_label(branch, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_compatible_signed_ordered_branch_slots(
        &mut self,
        op: CompareOp,
        left: StorageSlot,
        right: StorageSlot,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, branch_opcode)) = op.signed_order_branch_plan() else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        self.emit_compare_slot_slot_byte(cmp_left, cmp_right, 0);
        self.emit_lda_slot_byte(cmp_left, 1);
        self.emit_sbc_slot_byte(cmp_right, 1);
        self.preserve_y_one_for_branch_target(label);
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::SignedOrder, label, span);
        true
    }

    pub(super) fn emit_modern_signed_zero_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        let Some(compare_op) = CompareOp::from_binary(op) else {
            return false;
        };
        if compare_op.signed_order_branch_plan().is_none() {
            return false;
        }
        if self.constant_u16(right) == Some(0) && self.expr_signed(left) {
            return self.emit_modern_signed_expr_zero_branch(compare_op, left, label, span);
        }
        if self.constant_u16(left) == Some(0) && self.expr_signed(right) {
            return self.emit_modern_signed_expr_zero_branch(
                compare_op.reversed_operands(),
                right,
                label,
                span,
            );
        }
        false
    }

    pub(super) fn emit_modern_signed_expr_zero_branch(
        &mut self,
        op: CompareOp,
        expr: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(slot) = self.lvalue_slot(expr) else {
            return false;
        };
        self.emit_modern_signed_zero_slot_branch(op, slot, label, span)
    }

    pub(super) fn emit_modern_signed_zero_slot_branch(
        &mut self,
        op: CompareOp,
        slot: StorageSlot,
        label: &str,
        span: Span,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        if slot.size != 2 || !slot.signed {
            return false;
        }

        match op {
            CompareOp::Lt => {
                self.emit_lda_slot_byte(slot, 1);
                self.preserve_y_one_for_branch_target(label);
                self.emit_compare_branch_label(
                    opcode::BMI_REL,
                    CompareBranchFlags::SignedOrder,
                    label,
                    span,
                );
                true
            }
            CompareOp::Ge => {
                self.emit_lda_slot_byte(slot, 1);
                self.preserve_y_one_for_branch_target(label);
                self.emit_compare_branch_label(
                    opcode::BPL_REL,
                    CompareBranchFlags::SignedOrder,
                    label,
                    span,
                );
                true
            }
            CompareOp::Gt => {
                let done_label = self.next_label("compare:done");
                self.emit_lda_slot_byte(slot, 1);
                self.emit_compare_branch_label(
                    opcode::BMI_REL,
                    CompareBranchFlags::SignedOrder,
                    &done_label,
                    span,
                );
                self.emit_compare_branch_label(
                    opcode::BNE_REL,
                    CompareBranchFlags::Equality,
                    label,
                    span,
                );
                self.emit_lda_slot_byte(slot, 0);
                self.preserve_y_one_for_branch_target(label);
                self.emit_compare_branch_label(
                    opcode::BNE_REL,
                    CompareBranchFlags::Equality,
                    label,
                    span,
                );
                self.bind_codegen_label(done_label, span);
                true
            }
            CompareOp::Le => {
                let done_label = self.next_label("compare:done");
                self.emit_lda_slot_byte(slot, 1);
                self.emit_compare_branch_label(
                    opcode::BMI_REL,
                    CompareBranchFlags::SignedOrder,
                    label,
                    span,
                );
                self.emit_compare_branch_label(
                    opcode::BNE_REL,
                    CompareBranchFlags::Equality,
                    &done_label,
                    span,
                );
                self.emit_lda_slot_byte(slot, 0);
                self.preserve_y_one_for_branch_target(label);
                self.emit_compare_branch_label(
                    opcode::BEQ_REL,
                    CompareBranchFlags::Equality,
                    label,
                    span,
                );
                self.bind_codegen_label(done_label, span);
                true
            }
            CompareOp::Eq | CompareOp::Ne => false,
        }
    }

    pub(super) fn emit_signed_zero_lvalue_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some(compare_op) = CompareOp::from_binary(op) else {
            return false;
        };
        let Some(branch) = compare_op.signed_zero_lvalue_branch() else {
            return false;
        };
        if self.constant_u16(right) != Some(0) || !self.expr_signed(left) {
            return false;
        }
        let Some(slot) = self.lvalue_slot(left) else {
            return false;
        };
        if slot.size != 2 || !slot.signed {
            return false;
        }
        if !self.emit_compare_slot_expr_byte(slot, right, 0) {
            return false;
        }
        self.emit_lda_slot_byte(slot, 1);
        if !self.emit_sub_simple_byte(right, 1) {
            return false;
        }
        self.preserve_y_one_for_branch_target(label);
        self.emit_compare_branch_label(branch, CompareBranchFlags::SignedOrder, label, span);
        true
    }

    pub(super) fn emit_ordered_branch_slot_expr(
        &mut self,
        op: CompareOp,
        left: StorageSlot,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, include_equal)) = op.ordered_operand_plan() else {
            return false;
        };
        if swap_operands {
            let right_slot = StorageSlot::zero_page(runtime_zp::ADDR.address(), width);
            if !self.emit_expr_to_slot(right, right_slot) {
                return false;
            }
            let op = if include_equal {
                CompareOp::Le
            } else {
                CompareOp::Lt
            };
            return self.emit_ordered_branch_slots(op, right_slot, left, width, label, span);
        }
        if include_equal {
            self.emit_le_branch_slot_expr(left, right, width, label, span)
        } else {
            self.emit_lt_branch_slot_expr(left, right, width, label, span)
        }
    }

    pub(super) fn emit_lt_branch_slot_expr(
        &mut self,
        left: StorageSlot,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            if !self.emit_compare_slot_expr_byte(left, right, byte_index) {
                return false;
            }
            self.emit_unsigned_ordered_branch_step(false, byte_index, label, &done_label, span);
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_le_branch_slot_expr(
        &mut self,
        left: StorageSlot,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            if !self.emit_compare_slot_expr_byte(left, right, byte_index) {
                return false;
            }
            self.emit_unsigned_ordered_branch_step(true, byte_index, label, &done_label, span);
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_ordered_branch_slots(
        &mut self,
        op: CompareOp,
        left: StorageSlot,
        right: StorageSlot,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, include_equal)) = op.ordered_operand_plan() else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        if include_equal {
            self.emit_le_branch_slots(cmp_left, cmp_right, width, label, span)
        } else {
            self.emit_lt_branch_slots(cmp_left, cmp_right, width, label, span)
        }
    }

    pub(super) fn emit_lt_branch_slots(
        &mut self,
        left: StorageSlot,
        right: StorageSlot,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            self.emit_compare_slot_slot_byte(left, right, byte_index);
            self.emit_unsigned_ordered_branch_step(false, byte_index, label, &done_label, span);
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_le_branch_slots(
        &mut self,
        left: StorageSlot,
        right: StorageSlot,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            self.emit_compare_slot_slot_byte(left, right, byte_index);
            self.emit_unsigned_ordered_branch_step(true, byte_index, label, &done_label, span);
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_unsigned_ordered_branch_step(
        &mut self,
        include_equal: bool,
        byte_index: u16,
        true_label: &str,
        done_label: &str,
        span: Span,
    ) {
        self.emit_compare_branch_label(
            opcode::BCC_REL,
            CompareBranchFlags::UnsignedOrder,
            true_label,
            span,
        );
        if include_equal && byte_index == 0 {
            self.emit_compare_branch_label(
                opcode::BEQ_REL,
                CompareBranchFlags::UnsignedOrder,
                true_label,
                span,
            );
        } else if byte_index > 0 {
            self.emit_compare_branch_label(
                opcode::BNE_REL,
                CompareBranchFlags::UnsignedOrder,
                done_label,
                span,
            );
        }
    }

    pub(super) fn emit_equality_branch_step(
        &mut self,
        op: CompareOp,
        true_label: &str,
        done_label: &str,
        span: Span,
    ) {
        let target = match op {
            CompareOp::Eq => done_label,
            CompareOp::Ne => true_label,
            _ => return,
        };
        self.emit_compare_branch_label(opcode::BNE_REL, CompareBranchFlags::Equality, target, span);
    }

    pub(super) fn emit_byte_eq_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        if self.constant_u16(right) == Some(0) {
            return self.emit_byte_zero_branch(left, label, opcode::BEQ_REL, span);
        }
        if self.constant_u16(left) == Some(0) {
            return self.emit_byte_zero_branch(right, label, opcode::BEQ_REL, span);
        }
        if self.profile.enables_modern_optimizations() {
            if !self.emit_compare_simple_byte(left, right, 0) {
                return false;
            }
        } else {
            if !self.emit_load_simple_byte(left, 0) {
                return false;
            }
            if !self.emit_xor_simple_byte(right, 0) {
                return false;
            }
        }
        if self.straight_line_store_y == Some(1) {
            self.label_store_y_hints.insert(label.to_string(), 1);
        }
        self.emit_compare_branch_label(opcode::BEQ_REL, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_byte_ne_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        if self.constant_u16(right) == Some(0) {
            return self.emit_byte_zero_branch(left, label, opcode::BNE_REL, span);
        }
        if self.constant_u16(left) == Some(0) {
            return self.emit_byte_zero_branch(right, label, opcode::BNE_REL, span);
        }
        if self.profile.enables_modern_optimizations() {
            if !self.emit_compare_simple_byte(left, right, 0) {
                return false;
            }
        } else {
            if !self.emit_load_simple_byte(left, 0) {
                return false;
            }
            if !self.emit_xor_simple_byte(right, 0) {
                return false;
            }
        }
        self.preserve_y_one_for_branch_target(label);
        self.emit_compare_branch_label(opcode::BNE_REL, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_byte_zero_branch(
        &mut self,
        expr: &Expr,
        label: &str,
        branch_opcode: u8,
        span: Span,
    ) -> bool {
        if !self.emit_load_simple_byte(expr, 0) {
            return false;
        }
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn preserve_y_one_for_branch_target(&mut self, label: &str) {
        if self.straight_line_store_y == Some(1) {
            self.label_store_y_hints.insert(label.to_string(), 1);
        }
    }

    pub(super) fn emit_compare_branch_label(
        &mut self,
        opcode: u8,
        flags: CompareBranchFlags,
        label: &str,
        span: Span,
    ) {
        debug_assert_compare_branch_opcode(opcode, flags);
        self.emitter.emit_branch_label(opcode, label, span);
    }

    pub(super) fn emit_word_eq_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        if self.constant_u16(right) == Some(0) {
            return self.emit_word_zero_branch(left, label, opcode::BEQ_REL, span);
        }
        if self.constant_u16(left) == Some(0) {
            return self.emit_word_zero_branch(right, label, opcode::BEQ_REL, span);
        }
        if self.emit_compatible_reused_word_equality_branch(
            left,
            right,
            label,
            opcode::BEQ_REL,
            span,
        ) {
            return true;
        }
        if self.emit_word_eq_branch_with_prepared_rhs(left, right, label, opcode::BEQ_REL, span) {
            return true;
        }
        let done_label = self.next_label("compare:done");
        if !self.emit_load_simple_byte(left, 0) {
            return false;
        }
        if !self.emit_xor_simple_byte(right, 0) {
            return false;
        }
        self.emit_compare_branch_label(
            opcode::BNE_REL,
            CompareBranchFlags::Equality,
            &done_label,
            span,
        );
        if !self.emit_or_simple_byte(left, 1) {
            return false;
        }
        if !self.emit_xor_simple_byte(right, 1) {
            return false;
        }
        self.bind_codegen_label(done_label, span);
        self.emit_compare_branch_label(opcode::BEQ_REL, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_word_ne_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        if self.constant_u16(right) == Some(0) {
            return self.emit_word_zero_branch(left, label, opcode::BNE_REL, span);
        }
        if self.constant_u16(left) == Some(0) {
            return self.emit_word_zero_branch(right, label, opcode::BNE_REL, span);
        }
        if self.emit_compatible_reused_word_equality_branch(
            left,
            right,
            label,
            opcode::BNE_REL,
            span,
        ) {
            return true;
        }
        if self.emit_word_eq_branch_with_prepared_rhs(left, right, label, opcode::BNE_REL, span) {
            return true;
        }
        let done_label = self.next_label("compare:done");
        if !self.emit_load_simple_byte(left, 0) {
            return false;
        }
        if !self.emit_xor_simple_byte(right, 0) {
            return false;
        }
        self.emit_compare_branch_label(
            opcode::BNE_REL,
            CompareBranchFlags::Equality,
            &done_label,
            span,
        );
        if !self.emit_or_simple_byte(left, 1) {
            return false;
        }
        if !self.emit_xor_simple_byte(right, 1) {
            return false;
        }
        self.bind_codegen_label(done_label, span);
        self.emit_compare_branch_label(opcode::BNE_REL, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_word_eq_branch_with_prepared_rhs(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        branch_opcode: u8,
        span: Span,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() || self.direct_scalar_slot(left).is_none() {
            return false;
        }
        let Some(right_slot) = self.prepare_compare_rhs_slot(right) else {
            return false;
        };
        if right_slot.size < 2 {
            return false;
        }

        let done_label = self.next_label("compare:done");
        if !self.emit_load_simple_byte(left, 0) {
            return false;
        }
        self.emit_eor_slot_byte(right_slot, 0);
        self.emit_compare_branch_label(
            opcode::BNE_REL,
            CompareBranchFlags::Equality,
            &done_label,
            span,
        );
        if !self.emit_or_simple_byte(left, 1) {
            return false;
        }
        self.emit_eor_slot_byte(right_slot, 1);
        self.bind_codegen_label(done_label, span);
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_compatible_reused_word_equality_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        branch_opcode: u8,
        span: Span,
    ) -> bool {
        if !self.segment_storage {
            return false;
        }
        let Some(right_slot) = self.direct_scalar_slot(right) else {
            return false;
        };
        let slot = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR);
        let Some(left_slot) = slot.filter(|slot| slot.size >= 2) else {
            return false;
        };

        let done_label = self.next_label("compare:done");
        self.emit_lda_slot_byte(left_slot, 0);
        self.emit_eor_slot_byte(right_slot, 0);
        self.emit_compare_branch_label(
            opcode::BNE_REL,
            CompareBranchFlags::Equality,
            &done_label,
            span,
        );
        self.emit_ora_slot_byte(left_slot, 1);
        if right_slot.size > 1 {
            self.emit_eor_slot_byte(right_slot, 1);
        }
        self.bind_codegen_label(done_label, span);
        if branch_opcode == opcode::BEQ_REL && left_slot.space == AddressSpace::IndirectIndexedY {
            self.label_store_y_hints.insert(label.to_string(), 1);
        }
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_word_zero_branch(
        &mut self,
        expr: &Expr,
        label: &str,
        branch_opcode: u8,
        span: Span,
    ) -> bool {
        if self.segment_storage && Self::arithmetic_operand_needs_materialization(expr) {
            let slot = self.reusable_lvalue_slot_with_pointer(expr, runtime_zp::ARRAY_ADDR);
            if let Some(slot) = slot.filter(|slot| slot.size >= 2) {
                self.emit_lda_slot_byte(slot, 0);
                self.emit_ora_slot_byte(slot, 1);
                self.emit_compare_branch_label(
                    branch_opcode,
                    CompareBranchFlags::Equality,
                    label,
                    span,
                );
                return true;
            }
        }
        if !self.emit_load_simple_byte(expr, 0) {
            return false;
        }
        if !self.emit_or_simple_byte(expr, 1) {
            return false;
        }
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::Equality, label, span);
        true
    }

    pub(super) fn emit_compatible_byte_ordered_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, branch_opcode)) =
            CompareOp::from_binary(op).and_then(CompareOp::unsigned_order_branch_plan)
        else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        if !self.emit_compare_simple_byte(cmp_left, cmp_right, 0) {
            return false;
        }
        self.emit_compare_branch_label(
            branch_opcode,
            CompareBranchFlags::UnsignedOrder,
            label,
            span,
        );
        true
    }

    pub(super) fn is_signed_compare(&self, left: &Expr, right: &Expr, width: u16) -> bool {
        width == 2 && (self.expr_signed(left) || self.expr_signed(right))
    }

    pub(super) fn emit_compatible_signed_ordered_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, branch_opcode)) =
            CompareOp::from_binary(op).and_then(CompareOp::signed_order_branch_plan)
        else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        if self.constant_u16(cmp_left) == Some(0)
            && self.emit_compatible_signed_zero_indirect_branch(
                cmp_right,
                branch_opcode,
                label,
                span,
            )
        {
            return true;
        }
        if !self.emit_compare_simple_byte(cmp_left, cmp_right, 0) {
            return false;
        }
        if !self.emit_load_simple_byte(cmp_left, 1) {
            return false;
        }
        if !self.emit_sub_simple_byte(cmp_right, 1) {
            return false;
        }
        self.preserve_y_one_for_branch_target(label);
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::SignedOrder, label, span);
        true
    }

    pub(super) fn emit_compatible_signed_zero_indirect_branch(
        &mut self,
        right: &Expr,
        branch_opcode: u8,
        label: &str,
        span: Span,
    ) -> bool {
        if !self.segment_storage || !self.expr_signed(right) {
            return false;
        }
        let Some(slot) = self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if slot.space != AddressSpace::IndirectIndexedY || slot.size != 2 || !slot.signed {
            return false;
        }

        self.emit_lda_imm(0);
        self.emit_cmp_slot_byte(slot, 0);
        self.emit_lda_imm(0);
        self.emit_sbc_slot_byte(slot, 1);
        self.preserve_y_one_for_branch_target(label);
        self.emit_compare_branch_label(branch_opcode, CompareBranchFlags::SignedOrder, label, span);
        true
    }

    pub(super) fn emit_compatible_unsigned_subtract_compare(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> bool {
        if self.segment_storage
            && self.expr_size(left).is_some_and(|size| size >= 2)
            && let Some(value) = self.constant_u16(right)
        {
            let slot = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR);
            if let Some(slot) = slot.filter(|slot| slot.size >= 2) {
                let immediate = Immediate::new(value);
                self.emit_lda_slot_byte(slot, 0);
                self.emit_cmp_immediate(immediate, 0);
                self.emit_lda_slot_byte(slot, 1);
                self.emit_sbc_immediate(immediate, 1);
                return true;
            }
        }
        if self.segment_storage
            && self.expr_size(left).is_some_and(|size| size >= 2)
            && let Some(right_slot) = self.direct_scalar_slot(right)
        {
            let slot = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR);
            if let Some(left_slot) = slot.filter(|slot| slot.size >= 2) {
                self.emit_lda_slot_byte(left_slot, 0);
                self.emit_cmp_slot_byte(right_slot, 0);
                self.emit_lda_slot_byte(left_slot, 1);
                if right_slot.size > 1 {
                    self.emit_sbc_slot_byte(right_slot, 1);
                } else {
                    self.emit_sbc_imm(0);
                }
                return true;
            }
        }
        if self.segment_storage
            && self.expr_size(right).is_some_and(|size| size >= 2)
            && let Some(value) = self.constant_u16(left)
        {
            let slot = self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ARRAY_ADDR);
            if let Some(slot) = slot.filter(|slot| slot.size >= 2) {
                let immediate = Immediate::new(value);
                self.emit_lda_immediate(immediate, 0);
                self.emit_cmp_slot_byte(slot, 0);
                self.emit_lda_immediate(immediate, 1);
                self.emit_sbc_slot_byte(slot, 1);
                return true;
            }
        }
        if !self.emit_compare_simple_byte(left, right, 0) {
            return false;
        }
        if !self.emit_load_simple_byte(left, 1) {
            return false;
        }
        self.emit_sub_simple_byte(right, 1)
    }

    pub(super) fn emit_compatible_unsigned_ordered_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, branch_opcode)) =
            CompareOp::from_binary(op).and_then(CompareOp::unsigned_order_branch_plan)
        else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        if !self.emit_compatible_unsigned_subtract_compare(cmp_left, cmp_right) {
            return false;
        }
        self.emit_compare_branch_label(
            branch_opcode,
            CompareBranchFlags::UnsignedOrder,
            label,
            span,
        );
        true
    }

    pub(super) fn emit_eq_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            if !self.emit_compare_simple_byte(left, right, byte_index) {
                return false;
            }
            self.emit_equality_branch_step(CompareOp::Eq, label, &done_label, span);
        }
        self.emit_jmp_label(label, span);
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_ne_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        for byte_index in (0..width).rev() {
            if !self.emit_compare_simple_byte(left, right, byte_index) {
                return false;
            }
            self.emit_equality_branch_step(CompareOp::Ne, label, label, span);
        }
        true
    }

    pub(super) fn emit_ordered_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, include_equal)) =
            CompareOp::from_binary(op).and_then(CompareOp::ordered_operand_plan)
        else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        if include_equal {
            self.emit_le_branch(cmp_left, cmp_right, width, label, span)
        } else {
            self.emit_lt_branch(cmp_left, cmp_right, width, label, span)
        }
    }

    pub(super) fn emit_signed_ordered_branch(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let Some((swap_operands, include_equal)) =
            CompareOp::from_binary(op).and_then(CompareOp::ordered_operand_plan)
        else {
            return false;
        };
        let (cmp_left, cmp_right) = if swap_operands {
            (right, left)
        } else {
            (left, right)
        };
        if include_equal {
            self.emit_signed_le_branch(cmp_left, cmp_right, label, span)
        } else {
            self.emit_signed_lt_branch(cmp_left, cmp_right, label, span)
        }
    }

    pub(super) fn emit_signed_lt_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let same_sign_label = self.next_label("signed:same");
        let done_label = self.next_label("signed:done");

        if !self.emit_load_simple_byte(left, 1) {
            return false;
        }
        if !self.emit_xor_simple_byte(right, 1) {
            return false;
        }
        self.emit_compare_branch_label(
            opcode::BPL_REL,
            CompareBranchFlags::SignedOrder,
            &same_sign_label,
            span,
        );

        if !self.emit_load_simple_byte(left, 1) {
            return false;
        }
        self.emit_compare_branch_label(
            opcode::BMI_REL,
            CompareBranchFlags::SignedOrder,
            label,
            span,
        );
        self.emit_jmp_label(&done_label, span);

        self.bind_codegen_label(same_sign_label, span);
        if !self.emit_lt_branch(left, right, 2, label, span) {
            return false;
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_signed_le_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        label: &str,
        span: Span,
    ) -> bool {
        let false_label = self.next_label("signed:false");
        if !self.emit_signed_lt_branch(right, left, &false_label, span) {
            return false;
        }
        self.emit_jmp_label(label, span);
        self.bind_codegen_label(false_label, span);
        true
    }

    pub(super) fn emit_lt_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            if !self.emit_compare_simple_byte(left, right, byte_index) {
                return false;
            }
            self.emit_unsigned_ordered_branch_step(false, byte_index, label, &done_label, span);
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_le_branch(
        &mut self,
        left: &Expr,
        right: &Expr,
        width: u16,
        label: &str,
        span: Span,
    ) -> bool {
        let done_label = self.next_label("compare:done");
        for byte_index in (0..width).rev() {
            if !self.emit_compare_simple_byte(left, right, byte_index) {
                return false;
            }
            self.emit_unsigned_ordered_branch_step(true, byte_index, label, &done_label, span);
        }
        self.bind_codegen_label(done_label, span);
        true
    }

    pub(super) fn emit_load_simple_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        if let ExprKind::Cast { expr, .. } = &expr.kind {
            return self.emit_load_simple_byte(expr, byte_index);
        }

        if let Some(value) = self.constant_u16(expr) {
            self.emit_lda_immediate(Immediate::new(value), byte_index);
            return true;
        }

        if self.emit_inline_byte_array_call_index_load(expr, byte_index) {
            return true;
        }

        if self.emit_inline_byte_array_scalar_index_load(expr, byte_index) {
            return true;
        }

        if self.emit_load_effective_address_byte(expr, byte_index) {
            return true;
        }

        if self.profile.enables_modern_optimizations()
            && self.emit_proven_call_result_byte(expr, byte_index)
        {
            return true;
        }

        if let ExprKind::Call { callee, args } = &expr.kind
            && self.array_call_slot_size(callee, args).is_none()
            && let Some(return_slot) = self.call_return_slot(callee)
        {
            if !self.emit_call(callee, args, expr.span) {
                return false;
            }
            if byte_index >= return_slot.size {
                self.emit_lda_imm(0);
            } else {
                self.emit_lda_slot_byte(return_slot, byte_index);
            }
            return true;
        }

        if let ExprKind::Unary {
            op: UnaryOp::AddressOf,
            expr,
        } = &expr.kind
        {
            if let ExprKind::Name(name) = &expr.kind
                && self.emit_load_routine_address_byte(name, byte_index, expr.span)
            {
                return true;
            }
            let Some(address) = self.address_of_lvalue(expr) else {
                return false;
            };
            self.emit_lda_immediate(Immediate::new(address.address()), byte_index);
            return true;
        }

        if self.emit_load_array_pointer_value_byte(expr, byte_index) {
            return true;
        }

        if self.profile.enables_modern_optimizations()
            && self.emit_proven_simple_value_byte(expr, byte_index)
        {
            return true;
        }

        let Some(slot) = self
            .reusable_prepared_lvalue_slot(expr)
            .or_else(|| self.lvalue_slot(expr))
        else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_lda_imm(0);
        } else {
            self.emit_lda_slot_byte(slot, byte_index);
        }
        true
    }

    fn emit_load_routine_address_byte(&mut self, name: &str, byte_index: u16, span: Span) -> bool {
        if byte_index > 1 {
            self.emit_lda_imm(0);
            return true;
        }
        let Some(routine) = self.routines.get(&normalize_name(name)).cloned() else {
            return false;
        };
        if let Some(address) = routine.system_address {
            self.emit_lda_immediate(Immediate::new(address), byte_index);
            return true;
        }
        if byte_index == 0 {
            self.emit_lda_label_low(routine.label, span);
        } else {
            self.emit_lda_label_high(routine.label, span);
        }
        true
    }

    pub(super) fn emit_proven_call_result_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        let ExprKind::Call { callee, args } = &expr.kind else {
            return false;
        };
        if self.array_call_slot_size(callee, args).is_some() {
            return false;
        }
        let proof = self.value_availability_proof(expr);
        if proof.source != ValueAvailabilitySource::RoutineReturn {
            return false;
        }
        if proof.width.is_none() {
            return false;
        }
        if !self.emit_call(callee, args, expr.span) {
            return false;
        }
        self.emit_proven_value_byte_from_proof(&proof, byte_index, expr.span)
    }

    pub(super) fn emit_proven_simple_value_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        let proof = self.value_availability_proof(expr);
        if !matches!(
            proof.source,
            ValueAvailabilitySource::Constant | ValueAvailabilitySource::Storage
        ) {
            self.record_codegen_proof_rejection(
                "value-availability",
                expr.span,
                "simple byte load requires constant or storage source",
            );
            return false;
        }
        self.emit_proven_value_byte_from_proof(&proof, byte_index, expr.span)
    }

    fn emit_proven_value_byte_from_proof(
        &mut self,
        proof: &ValueAvailabilityProof,
        byte_index: u16,
        span: Span,
    ) -> bool {
        let Some(width) = proof.width else {
            self.record_codegen_proof_rejection(
                "value-availability",
                span,
                "value width is unknown",
            );
            return false;
        };
        if byte_index >= width {
            self.emit_lda_imm(0);
            return true;
        }
        match proof.bytes.get(usize::from(byte_index)).copied().flatten() {
            Some(ValueByteAvailability::Register(RegisterName::A)) => {
                self.record_codegen_proof(
                    "value-availability",
                    span,
                    "call result byte is already available in A",
                );
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallResultMaterializationRemoved,
                    slot_load_instruction_len(StorageSlot::zero_page(
                        runtime_zp::ARGS.offset(byte_index as u8).address(),
                        1,
                    )),
                    Some(span),
                    "used proven accumulator call result byte",
                );
                true
            }
            Some(ValueByteAvailability::Register(RegisterName::X)) => {
                self.emit_txa();
                true
            }
            Some(ValueByteAvailability::Register(RegisterName::Y)) => {
                self.emit_tya();
                true
            }
            Some(ValueByteAvailability::PublicReturnSlot { slot, byte_index }) => {
                self.emit_lda_slot_byte(slot, byte_index);
                true
            }
            Some(ValueByteAvailability::Slot { slot, byte_index }) => {
                self.emit_lda_slot_byte(slot, byte_index);
                true
            }
            Some(ValueByteAvailability::Constant(value)) => {
                self.emit_lda_imm(value);
                true
            }
            None => {
                self.record_codegen_proof_rejection(
                    "value-availability",
                    span,
                    format!("byte {byte_index} is not available"),
                );
                false
            }
        }
    }

    pub(super) fn emit_inline_byte_array_call_index_load(
        &mut self,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        if byte_index != 0 {
            return false;
        }
        let Some((array, index)) = self.inline_byte_array_call_index(expr) else {
            return false;
        };
        let ExprKind::Call { callee, args } = &index.kind else {
            return false;
        };
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };
        if return_slot.size != 1 || !self.emit_call(callee, args, index.span) {
            return false;
        }
        self.emit_ldx_slot_byte(return_slot, 0);
        self.emitter
            .emit_lda_absolute_x(AbsoluteX::new(array.address));
        true
    }

    pub(super) fn emit_inline_byte_array_scalar_index_load(
        &mut self,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        if byte_index != 0 || !self.profile.enables_modern_optimizations() {
            return false;
        }
        let Some(proof) = self.index_address_proof(expr) else {
            self.record_codegen_proof_rejection(
                "index-address",
                expr.span,
                "missing index-address proof",
            );
            return false;
        };
        if proof.mode != IndexAddressMode::AbsoluteY || proof.element_size != 1 {
            self.record_codegen_proof_rejection(
                "index-address",
                expr.span,
                format!(
                    "requires absolute,Y byte element, got mode {:?} element_size {}",
                    proof.mode, proof.element_size
                ),
            );
            return false;
        }
        let Some(index_expr) = indexed_expr_index(expr) else {
            self.record_codegen_proof_rejection(
                "index-address",
                expr.span,
                "indexed expression has no index operand",
            );
            return false;
        };
        let Some(index) = self.direct_scalar_slot(index_expr) else {
            self.record_codegen_proof_rejection(
                "index-address",
                expr.span,
                "index operand is not direct scalar storage",
            );
            return false;
        };
        if index.size != 1 {
            self.record_codegen_proof_rejection(
                "index-address",
                expr.span,
                format!("index storage is {} bytes, not byte-sized", index.size),
            );
            return false;
        }
        self.emit_ldy_slot_byte(index, 0);
        self.emit_lda_absolute_y(Absolute::new(proof.base.address));
        self.record_codegen_proof(
            "index-address",
            expr.span,
            "inline byte array scalar index proved as absolute,Y",
        );
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressLowered,
            0,
            Some(expr.span),
            "lowered inline byte array index through proof-guided absolute,Y",
        );
        true
    }
}

impl Generator {
    pub(super) fn emit_compare_simple_byte(
        &mut self,
        left: &Expr,
        right: &Expr,
        byte_index: u16,
    ) -> bool {
        if let Some(value) = self.constant_u16(right) {
            if !self.emit_load_simple_byte(left, byte_index) {
                return false;
            }
            self.emit_cmp_immediate(Immediate::new(value), byte_index);
            return true;
        }

        if let Some(value) = self.constant_u16(left) {
            let Some(slot) = self
                .reusable_prepared_lvalue_slot(right)
                .or_else(|| self.lvalue_slot(right))
            else {
                return false;
            };
            self.emit_lda_immediate(Immediate::new(value), byte_index);
            if byte_index >= slot.size {
                self.emit_cmp_imm(0);
            } else {
                self.emit_cmp_slot_byte(slot, byte_index);
            }
            return true;
        }

        if let Some(slot) = self.prepare_compare_rhs_slot(right) {
            if !self.emit_load_simple_byte(left, byte_index) {
                return false;
            }
            if byte_index >= slot.size {
                self.emit_cmp_imm(0);
            } else {
                self.emit_cmp_slot_byte(slot, byte_index);
            }
            return true;
        }

        if !self.emit_load_simple_byte(left, byte_index) {
            return false;
        }

        let Some(slot) = self
            .reusable_prepared_lvalue_slot(right)
            .or_else(|| self.lvalue_slot(right))
        else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_cmp_imm(0);
        } else {
            self.emit_cmp_slot_byte(slot, byte_index);
        }
        true
    }

    pub(super) fn prepare_compare_rhs_slot(&mut self, right: &Expr) -> Option<StorageSlot> {
        if !self.profile.enables_modern_optimizations()
            || self.prepared_pointer_fact(right).is_none()
        {
            return None;
        }
        if let Some(slot) = self.reusable_prepared_lvalue_slot(right) {
            return Some(slot);
        }
        self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ELEMENT_ADDR)
    }

    fn emit_compare_slot_expr_byte(
        &mut self,
        left: StorageSlot,
        right: &Expr,
        byte_index: u16,
    ) -> bool {
        self.emit_lda_slot_byte(left, byte_index);
        if let Some(value) = self.constant_u16(right) {
            self.emit_cmp_immediate(Immediate::new(value), byte_index);
            return true;
        }

        let Some(slot) = self
            .reusable_prepared_lvalue_slot(right)
            .or_else(|| self.lvalue_slot(right))
        else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_cmp_imm(0);
        } else {
            self.emit_cmp_slot_byte(slot, byte_index);
        }
        true
    }

    pub(super) fn emit_compare_slot_slot_byte(
        &mut self,
        left: StorageSlot,
        right: StorageSlot,
        byte_index: u16,
    ) {
        self.emit_lda_slot_byte(left, byte_index);
        if byte_index >= right.size {
            self.emit_cmp_imm(0);
        } else {
            self.emit_cmp_slot_byte(right, byte_index);
        }
    }
}
