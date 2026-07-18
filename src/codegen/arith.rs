use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BinaryArithmeticOp {
    Add,
    Sub,
}

impl BinaryArithmeticOp {
    pub(super) fn from_binary(op: BinaryOp) -> Option<Self> {
        match op {
            BinaryOp::Add => Some(Self::Add),
            BinaryOp::Sub => Some(Self::Sub),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BinaryBitwiseOp {
    And,
    Or,
    Xor,
}

impl BinaryBitwiseOp {
    pub(super) fn from_binary(op: BinaryOp) -> Option<Self> {
        match op {
            BinaryOp::And => Some(Self::And),
            BinaryOp::Or => Some(Self::Or),
            BinaryOp::Xor => Some(Self::Xor),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BinaryScalarOp {
    Arithmetic(BinaryArithmeticOp),
    Bitwise(BinaryBitwiseOp),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum BinaryByteOperand<'a> {
    Expr(&'a Expr),
    Slot(StorageSlot),
    MissingZero,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum BinaryByteSource<'a> {
    Expr(&'a Expr),
    Slot(StorageSlot),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum BinaryCarryTiming {
    BeforeLeft,
    ExprExprCompatible,
}

enum AddressAddLowering<'a> {
    PointerPlusOwnByteDeref {
        pointer: StorageSlot,
        offset: u16,
    },
    ArrayPointerPlusByte {
        array: StorageSlot,
        addend: &'a Expr,
    },
}

impl BinaryScalarOp {
    pub(super) fn from_binary(op: BinaryOp) -> Option<Self> {
        if let Some(op) = BinaryArithmeticOp::from_binary(op) {
            return Some(Self::Arithmetic(op));
        }
        BinaryBitwiseOp::from_binary(op).map(Self::Bitwise)
    }

    pub(super) fn is_bitwise(self) -> bool {
        matches!(self, Self::Bitwise(_))
    }

    pub(super) fn allows_reversed_materialized_rhs(self) -> bool {
        !matches!(self, Self::Arithmetic(BinaryArithmeticOp::Sub))
    }

    pub(super) fn supports_expr_left_slot_right(self) -> bool {
        matches!(self, Self::Arithmetic(BinaryArithmeticOp::Sub))
    }
}

impl Generator {
    pub(super) fn emit_binary_slot_expr_to_slot(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        self.emit_binary_bytes_to_slot(target, |generator, byte_index, set_carry| {
            generator.emit_binary_slot_expr_byte(op, left, right, byte_index, set_carry)
        })
    }

    pub(super) fn emit_binary_expr_slot_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: StorageSlot,
        target: StorageSlot,
    ) -> bool {
        self.emit_binary_bytes_to_slot(target, |generator, byte_index, set_carry| {
            generator.emit_binary_expr_slot_byte(op, left, right, byte_index, set_carry)
        })
    }

    pub(super) fn emit_binary_slot_slot_to_slot(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: StorageSlot,
        target: StorageSlot,
    ) -> bool {
        self.emit_binary_bytes_to_slot(target, |generator, byte_index, set_carry| {
            generator.emit_binary_slot_slot_byte(op, left, right, byte_index, set_carry)
        })
    }

    pub(super) fn emit_binary_bytes_to_slot<F>(
        &mut self,
        target: StorageSlot,
        mut emit_byte: F,
    ) -> bool
    where
        F: FnMut(&mut Self, u16, bool) -> bool,
    {
        if !emit_byte(self, 0, true) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);

        if target.size > 1 {
            if !emit_byte(self, 1, false) {
                return false;
            }
            self.emit_sta_slot_byte(target, 1);
        }

        true
    }

    pub(super) fn emit_binary_slot_slot_byte(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: StorageSlot,
        byte_index: u16,
        set_carry: bool,
    ) -> bool {
        self.emit_binary_scalar_byte(
            op,
            BinaryByteSource::Slot(left),
            Self::binary_slot_operand(right, byte_index),
            byte_index,
            set_carry,
            BinaryCarryTiming::BeforeLeft,
        )
    }

    pub(super) fn emit_binary_slot_expr_byte(
        &mut self,
        op: BinaryOp,
        left: StorageSlot,
        right: &Expr,
        byte_index: u16,
        set_carry: bool,
    ) -> bool {
        self.emit_binary_scalar_byte(
            op,
            BinaryByteSource::Slot(left),
            BinaryByteOperand::Expr(right),
            byte_index,
            set_carry,
            BinaryCarryTiming::BeforeLeft,
        )
    }

    pub(super) fn emit_binary_expr_slot_byte(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: StorageSlot,
        byte_index: u16,
        set_carry: bool,
    ) -> bool {
        let Some(scalar_op) = BinaryScalarOp::from_binary(op) else {
            return false;
        };
        if !scalar_op.supports_expr_left_slot_right() {
            return false;
        }
        self.emit_binary_scalar_byte(
            op,
            BinaryByteSource::Expr(left),
            Self::binary_slot_operand(right, byte_index),
            byte_index,
            set_carry,
            BinaryCarryTiming::BeforeLeft,
        )
    }

    pub(super) fn emit_binary_expr_byte(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        byte_index: u16,
        set_carry: bool,
    ) -> bool {
        self.emit_binary_scalar_byte(
            op,
            BinaryByteSource::Expr(left),
            BinaryByteOperand::Expr(right),
            byte_index,
            set_carry,
            BinaryCarryTiming::ExprExprCompatible,
        )
    }

    pub(super) fn emit_binary_left_byte(
        &mut self,
        source: BinaryByteSource<'_>,
        byte_index: u16,
    ) -> bool {
        match source {
            BinaryByteSource::Expr(expr) => self.emit_load_simple_byte_value_only(expr, byte_index),
            BinaryByteSource::Slot(slot) => {
                self.emit_lda_slot_byte_value_only(slot, byte_index);
                true
            }
        }
    }

    pub(super) fn emit_binary_scalar_byte(
        &mut self,
        op: BinaryOp,
        left: BinaryByteSource<'_>,
        right: BinaryByteOperand<'_>,
        byte_index: u16,
        set_carry: bool,
        carry_timing: BinaryCarryTiming,
    ) -> bool {
        let Some(op) = BinaryScalarOp::from_binary(op) else {
            return false;
        };

        match carry_timing {
            BinaryCarryTiming::BeforeLeft => self.emit_scalar_carry_setup(op, set_carry),
            BinaryCarryTiming::ExprExprCompatible if set_carry && self.segment_storage => {
                self.emit_scalar_carry_setup(op, true)
            }
            BinaryCarryTiming::ExprExprCompatible => {}
        }

        if !self.emit_binary_left_byte(left, byte_index) {
            return false;
        }

        if matches!(carry_timing, BinaryCarryTiming::ExprExprCompatible)
            && set_carry
            && !self.segment_storage
        {
            self.emit_scalar_carry_setup(op, true);
        }

        self.emit_scalar_operand_byte(op, right, byte_index)
    }

    pub(super) fn emit_scalar_operand_byte(
        &mut self,
        op: BinaryScalarOp,
        operand: BinaryByteOperand<'_>,
        byte_index: u16,
    ) -> bool {
        match operand {
            BinaryByteOperand::Expr(expr) => self.emit_scalar_simple_byte(op, expr, byte_index),
            BinaryByteOperand::Slot(slot) => {
                self.emit_scalar_slot_byte(op, slot, byte_index);
                true
            }
            BinaryByteOperand::MissingZero => {
                self.emit_scalar_missing_byte(op);
                true
            }
        }
    }

    pub(super) fn emit_scalar_simple_byte(
        &mut self,
        op: BinaryScalarOp,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        match op {
            BinaryScalarOp::Arithmetic(op) => {
                self.emit_arithmetic_simple_byte(op, expr, byte_index)
            }
            BinaryScalarOp::Bitwise(op) => self.emit_bitwise_simple_byte(op, expr, byte_index),
        }
    }

    pub(super) fn emit_scalar_slot_byte(
        &mut self,
        op: BinaryScalarOp,
        slot: StorageSlot,
        byte_index: u16,
    ) {
        match op {
            BinaryScalarOp::Arithmetic(op) => self.emit_arithmetic_slot_byte(op, slot, byte_index),
            BinaryScalarOp::Bitwise(op) => self.emit_bitwise_slot_byte(op, slot, byte_index),
        }
    }

    pub(super) fn emit_scalar_missing_byte(&mut self, op: BinaryScalarOp) {
        match op {
            BinaryScalarOp::Arithmetic(op) => {
                self.emit_arithmetic_immediate(op, Immediate::new(0), 0)
            }
            BinaryScalarOp::Bitwise(op) => self.emit_bitwise_immediate(op, Immediate::new(0), 0),
        }
    }

    pub(super) fn emit_scalar_carry_setup(&mut self, op: BinaryScalarOp, set_carry: bool) {
        if let BinaryScalarOp::Arithmetic(op) = op
            && set_carry
        {
            self.emit_arithmetic_carry_setup(op);
        }
    }

    pub(super) fn binary_slot_operand(
        slot: StorageSlot,
        byte_index: u16,
    ) -> BinaryByteOperand<'static> {
        if byte_index >= slot.size {
            BinaryByteOperand::MissingZero
        } else {
            BinaryByteOperand::Slot(slot)
        }
    }

    pub(super) fn emit_arithmetic_carry_setup(&mut self, op: BinaryArithmeticOp) {
        match op {
            BinaryArithmeticOp::Add => self.emit_clc(),
            BinaryArithmeticOp::Sub => self.emit_sec(),
        }
    }

    pub(super) fn emit_arithmetic_immediate(
        &mut self,
        op: BinaryArithmeticOp,
        immediate: Immediate,
        byte_index: u16,
    ) {
        match op {
            BinaryArithmeticOp::Add => self.emit_adc_immediate(immediate, byte_index),
            BinaryArithmeticOp::Sub => self.emit_sbc_immediate(immediate, byte_index),
        }
    }

    pub(super) fn emit_arithmetic_slot_byte(
        &mut self,
        op: BinaryArithmeticOp,
        slot: StorageSlot,
        byte_index: u16,
    ) {
        match op {
            BinaryArithmeticOp::Add => self.emit_adc_slot_byte(slot, byte_index),
            BinaryArithmeticOp::Sub => self.emit_sbc_slot_byte(slot, byte_index),
        }
    }

    pub(super) fn emit_add_simple_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        self.emit_arithmetic_simple_byte(BinaryArithmeticOp::Add, expr, byte_index)
    }

    pub(super) fn emit_sub_simple_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        self.emit_arithmetic_simple_byte(BinaryArithmeticOp::Sub, expr, byte_index)
    }

    pub(super) fn emit_arithmetic_simple_byte(
        &mut self,
        op: BinaryArithmeticOp,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        if let Some(value) = self.constant_u16(expr) {
            self.emit_arithmetic_raw_immediate(op, Immediate::new(value), byte_index);
            return true;
        }
        if self.emit_arithmetic_array_pointer_value_byte(op, expr, byte_index) {
            return true;
        }

        let Some(slot) = self.lvalue_slot(expr) else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_arithmetic_zero(op);
        } else {
            self.emit_arithmetic_slot_byte(op, slot, byte_index);
        }
        true
    }

    pub(super) fn emit_bitwise_simple_byte(
        &mut self,
        op: BinaryBitwiseOp,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        if let Some(value) = self.constant_u16(expr) {
            self.emit_bitwise_immediate(op, Immediate::new(value), byte_index);
            return true;
        }
        let Some(slot) = self.lvalue_slot(expr) else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_bitwise_missing_simple_byte(op);
        } else {
            self.emit_bitwise_slot_byte(op, slot, byte_index);
        }
        true
    }

    pub(super) fn emit_bitwise_immediate(
        &mut self,
        op: BinaryBitwiseOp,
        immediate: Immediate,
        byte_index: u16,
    ) {
        match op {
            BinaryBitwiseOp::And => self.emit_and_immediate(immediate, byte_index),
            BinaryBitwiseOp::Or => self.emit_ora_immediate(immediate, byte_index),
            BinaryBitwiseOp::Xor => self.emit_eor_immediate(immediate, byte_index),
        }
    }

    pub(super) fn emit_bitwise_missing_simple_byte(&mut self, op: BinaryBitwiseOp) {
        match op {
            BinaryBitwiseOp::And => self.emit_and_imm(0),
            BinaryBitwiseOp::Or | BinaryBitwiseOp::Xor => {}
        }
    }

    pub(super) fn emit_bitwise_slot_byte(
        &mut self,
        op: BinaryBitwiseOp,
        slot: StorageSlot,
        byte_index: u16,
    ) {
        match op {
            BinaryBitwiseOp::And => self.emit_and_slot_byte(slot, byte_index),
            BinaryBitwiseOp::Or => self.emit_ora_slot_byte(slot, byte_index),
            BinaryBitwiseOp::Xor => self.emit_eor_slot_byte(slot, byte_index),
        }
    }

    pub(super) fn emit_arithmetic_raw_immediate(
        &mut self,
        op: BinaryArithmeticOp,
        immediate: Immediate,
        byte_index: u16,
    ) {
        match op {
            BinaryArithmeticOp::Add => self.emit_adc_immediate(immediate, byte_index),
            BinaryArithmeticOp::Sub => self.emit_sbc_immediate(immediate, byte_index),
        }
    }

    pub(super) fn emit_arithmetic_zero(&mut self, op: BinaryArithmeticOp) {
        match op {
            BinaryArithmeticOp::Add => self.emit_adc_imm(0),
            BinaryArithmeticOp::Sub => self.emit_sbc_imm(0),
        }
    }

    pub(super) fn emit_arithmetic_array_pointer_value_byte(
        &mut self,
        op: BinaryArithmeticOp,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        let Some(slot) = self.array_pointer_value_slot(expr) else {
            return false;
        };
        match slot.array {
            Some(ArrayStorage::Inline) => {
                self.emit_arithmetic_raw_immediate(op, Immediate::new(slot.address), byte_index);
                true
            }
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) if byte_index < 2 => {
                match op {
                    BinaryArithmeticOp::Add => self
                        .emitter
                        .emit_adc_absolute(Absolute::new(slot.address.wrapping_add(byte_index))),
                    BinaryArithmeticOp::Sub => {
                        self.emit_sbc_absolute(Absolute::new(slot.address.wrapping_add(byte_index)))
                    }
                }
                true
            }
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) => {
                self.emit_arithmetic_zero(op);
                true
            }
            None => false,
        }
    }

    pub(super) fn array_pointer_value_slot(&self, expr: &Expr) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        self.lookup_slot(name).filter(|slot| slot.array.is_some())
    }

    pub(super) fn emit_or_simple_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        if let Some(value) = self.constant_u16(expr) {
            self.emitter
                .emit_ora_immediate(Immediate::new(value), byte_index);
            return true;
        }

        let Some(slot) = self.lvalue_slot(expr) else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_ora_imm(0);
        } else {
            self.emit_ora_slot_byte(slot, byte_index);
        }
        true
    }

    pub(super) fn emit_xor_simple_byte(&mut self, expr: &Expr, byte_index: u16) -> bool {
        if let Some(value) = self.constant_u16(expr) {
            self.emitter
                .emit_eor_immediate(Immediate::new(value), byte_index);
            return true;
        }

        let Some(slot) = self
            .reusable_prepared_lvalue_slot(expr)
            .or_else(|| self.lvalue_slot(expr))
        else {
            return false;
        };
        if byte_index >= slot.size {
            self.emit_eor_imm(0);
        } else {
            self.emit_eor_slot_byte(slot, byte_index);
        }
        true
    }

    pub(super) fn emit_shift_expr_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if let (Some(count), Some(left_size)) = (self.constant_u16(right), self.expr_size(left))
            && count >= left_size * 8
        {
            self.emit_store_constant(slot, 0);
            return true;
        }

        if self.segment_storage
            && let Some(count) = self.constant_u16(right)
            && self.direct_scalar_slot(left) == Some(slot)
            && self.emit_in_place_constant_shift(op, slot, count)
        {
            return true;
        }

        if self.segment_storage
            && let Some(count) = self.constant_u16(right)
            && slot.size == 1
            && self.expr_size(left).is_some_and(|size| size == 1)
            && (slot.space != AddressSpace::ZeroPage || self.inline_byte_constant_shift)
        {
            return match op {
                BinaryOp::Lsh => self.emit_lsh_expr_to_slot(left, slot, count),
                BinaryOp::Rsh => self.emit_rsh_expr_to_slot(left, slot, count),
                _ => false,
            };
        }

        if self.segment_storage {
            return self.emit_runtime_shift_expr_to_slot(op, left, right, slot);
        }

        let Some(count) = self.constant_u16(right) else {
            return self.emit_runtime_shift_expr_to_slot(op, left, right, slot);
        };
        let bit_width = slot.size * 8;
        if count >= bit_width {
            self.emit_store_constant(slot, 0);
            return true;
        }

        match op {
            BinaryOp::Lsh => self.emit_lsh_expr_to_slot(left, slot, count),
            BinaryOp::Rsh => self.emit_rsh_expr_to_slot(left, slot, count),
            _ => false,
        }
    }

    pub(super) fn emit_in_place_constant_shift(
        &mut self,
        op: BinaryOp,
        slot: StorageSlot,
        count: u16,
    ) -> bool {
        if slot.space != AddressSpace::Absolute || count == 0 {
            return false;
        }

        if slot.size == 1 {
            if self.profile.enables_modern_optimizations() && count <= 3 {
                for _ in 0..count {
                    self.emit_shift_absolute_byte_in_place(op, slot.absolute_byte(0), slot);
                }
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    6_i16.saturating_sub((count as i16) * 2),
                    None,
                    "shifted absolute byte in memory instead of reloading through accumulator",
                );
                return true;
            }
            self.emit_lda_slot_byte(slot, 0);
            for _ in 0..count {
                match op {
                    BinaryOp::Lsh => self.emit_asl_a(),
                    BinaryOp::Rsh => self.emit_lsr_a(),
                    _ => return false,
                }
            }
            self.emit_sta_slot_byte(slot, 0);
            return true;
        }

        if slot.size != 2 {
            return false;
        }

        for _ in 0..count {
            match op {
                BinaryOp::Lsh => {
                    self.emitter.emit_asl_absolute(slot.absolute_byte(0));
                    self.emitter.emit_rol_absolute(slot.absolute_byte(1));
                }
                BinaryOp::Rsh => {
                    self.emitter.emit_lsr_absolute(slot.absolute_byte(1));
                    self.emitter.emit_ror_absolute(slot.absolute_byte(0));
                }
                _ => return false,
            }
        }
        self.record_current_absolute_write(slot.absolute_byte(0).address(), 1);
        self.record_current_absolute_write(slot.absolute_byte(1).address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(slot.address, slot.size);
        self.processor.invalidate_memory_byte(slot, 0);
        self.processor.invalidate_memory_byte(slot, 1);
        self.processor.invalidate_value_flags();
        self.processor.invalidate_carry();
        true
    }

    pub(super) fn emit_shift_absolute_byte_in_place(
        &mut self,
        op: BinaryOp,
        absolute: Absolute,
        slot: StorageSlot,
    ) {
        match op {
            BinaryOp::Lsh => self.emitter.emit_asl_absolute(absolute),
            BinaryOp::Rsh => self.emitter.emit_lsr_absolute(absolute),
            _ => unreachable!("unsupported in-place byte shift"),
        }
        self.record_current_absolute_write(absolute.address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(absolute.address(), 1);
        if let Some(zero_page) = absolute_zero_page_alias(absolute) {
            self.processor.invalidate_zp(zero_page);
        }
        self.processor.invalidate_memory_byte(slot, 0);
        self.processor.invalidate_value_flags();
        self.processor.invalidate_carry();
    }

    pub(super) fn byte_constant_shift_parts<'a>(
        &self,
        expr: &'a Expr,
    ) -> Option<(BinaryOp, &'a Expr, u16)> {
        let ExprKind::Binary { op, left, right } = &expr.kind else {
            return None;
        };
        if !matches!(op, BinaryOp::Lsh | BinaryOp::Rsh) || self.expr_size(left)? != 1 {
            return None;
        }
        Some((*op, left, self.constant_u16(right)?))
    }

    pub(super) fn emit_byte_constant_shift_expr_to_slot(
        &mut self,
        expr: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if slot.size != 1 {
            return false;
        }
        let Some((op, left, count)) = self.byte_constant_shift_parts(expr) else {
            return false;
        };
        if count >= 8 {
            self.emit_lda_imm(0);
        } else if !self.emit_byte_constant_shift_to_acc(op, left, count) {
            return false;
        }
        self.emit_sta_slot_byte(slot, 0);
        true
    }

    pub(super) fn emit_byte_constant_shift_to_acc(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        count: u16,
    ) -> bool {
        if !self.emit_load_simple_byte_value_only(left, 0) {
            return false;
        }
        for _ in 0..count {
            match op {
                BinaryOp::Lsh => self.emit_asl_a(),
                BinaryOp::Rsh => self.emit_lsr_a(),
                _ => return false,
            }
        }
        true
    }

    pub(super) fn expr_is_effective_zero(&self, expr: &Expr) -> bool {
        if self.constant_u16(expr) == Some(0) {
            return true;
        }
        matches!(
            &expr.kind,
            ExprKind::Binary {
                op: BinaryOp::Lsh | BinaryOp::Rsh,
                left,
                right,
            } if self.constant_u16(right).zip(self.expr_size(left)).is_some_and(
                |(count, size)| count >= size * 8
            )
        )
    }

    pub(super) fn emit_runtime_binary_expr_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        slot: StorageSlot,
    ) -> bool {
        let helper = match op {
            BinaryOp::Mul => RuntimeHelperSlot::Mul,
            BinaryOp::Div => RuntimeHelperSlot::Div,
            BinaryOp::Mod => RuntimeHelperSlot::Mod,
            _ => return false,
        };
        self.emit_runtime_helper_expr_to_slot(
            helper,
            self.runtime_helpers.target(helper),
            left,
            right,
            slot,
            true,
        )
    }

    pub(super) fn emit_runtime_shift_expr_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        slot: StorageSlot,
    ) -> bool {
        let helper = match op {
            BinaryOp::Lsh => RuntimeHelperSlot::Lsh,
            BinaryOp::Rsh => RuntimeHelperSlot::Rsh,
            _ => return false,
        };
        if self.emit_runtime_byte_shift_lvalue_with_direct_count(
            helper,
            self.runtime_helpers.target(helper),
            left,
            right,
            slot,
        ) {
            return true;
        }
        self.emit_runtime_helper_expr_to_slot(
            helper,
            self.runtime_helpers.target(helper),
            left,
            right,
            slot,
            false,
        )
    }

    pub(super) fn emit_runtime_byte_shift_lvalue_with_direct_count(
        &mut self,
        helper_slot: RuntimeHelperSlot,
        helper: RuntimeHelperTarget,
        left: &Expr,
        right: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if !self.segment_storage
            || self.expr_size(left) != Some(1)
            || self
                .direct_scalar_slot(right)
                .is_none_or(|slot| slot.size != 1)
        {
            return false;
        }
        let left_slot = self.lvalue_slot(left);
        let Some(left_slot) = left_slot else {
            return false;
        };
        if left_slot.size != 1 || left_slot.space == AddressSpace::IndirectIndexedY {
            return false;
        }

        debug_assert_runtime_helper_abi_shape(helper_slot, &helper, slot, false);
        if !self.emit_load_simple_byte(right, 0) {
            return false;
        }
        self.emit_sta_zero_page(runtime_zp::AFCUR);
        self.emit_lda_slot_byte(left_slot, 0);
        self.emit_ldx_imm(0);
        self.emit_jsr_runtime_helper(helper_slot, helper, left.span);
        self.emit_sta_slot_byte(slot, 0);

        if slot.size > 1 {
            self.emit_txa();
            self.emit_sta_slot_byte(slot, 1);
        }

        true
    }

    pub(super) fn emit_runtime_helper_expr_to_slot(
        &mut self,
        helper_slot: RuntimeHelperSlot,
        helper: RuntimeHelperTarget,
        left: &Expr,
        right: &Expr,
        slot: StorageSlot,
        store_right_high: bool,
    ) -> bool {
        debug_assert_runtime_helper_abi_shape(helper_slot, &helper, slot, store_right_high);
        let materialized_left =
            if self.segment_storage && Self::arithmetic_operand_needs_materialization(left) {
                let temp_size = self.expr_size(left).unwrap_or(slot.size).min(slot.size);
                let temp = StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), temp_size);
                if !self.emit_expr_to_slot(left, temp) {
                    return false;
                }
                Some(temp)
            } else {
                None
            };
        let right_loaded_to_afcur = store_right_high
            && (self.emit_runtime_right_call_result_to_afcur(right)
                || self.emit_runtime_right_indexed_word_to_afcur(right));
        let right_loaded_to_afcur = right_loaded_to_afcur
            || (!store_right_high
                && self.segment_storage
                && self.emit_byte_constant_shift_expr_to_slot(
                    right,
                    StorageSlot::zero_page(runtime_zp::AFCUR.address(), 1),
                ));
        let materialized_right = if !right_loaded_to_afcur
            && self.segment_storage
            && Self::arithmetic_operand_needs_materialization(right)
        {
            let temp_size = self.expr_size(right).unwrap_or(slot.size).min(slot.size);
            let temp = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), temp_size);
            if !self.emit_expr_to_slot(right, temp) {
                return false;
            }
            Some(temp)
        } else {
            None
        };

        if store_right_high && !right_loaded_to_afcur {
            if let Some(right_slot) = materialized_right {
                if right_slot.size > 1 {
                    self.emit_lda_slot_byte(right_slot, 1);
                } else {
                    self.emit_lda_imm(0);
                }
            } else if !self.emit_load_simple_byte(right, 1) {
                return false;
            }
            self.emit_sta_zero_page(runtime_zp::AFCUR.offset(1));
        }

        if !right_loaded_to_afcur {
            if let Some(right_slot) = materialized_right {
                self.emit_lda_slot_byte(right_slot, 0);
            } else if !self.emit_load_simple_byte(right, 0) {
                return false;
            }
            self.emit_sta_zero_page(runtime_zp::AFCUR);
        }

        if self.segment_storage && self.constant_u16(left).is_some() {
            if !self.emit_load_simple_byte(left, 0) {
                return false;
            }
            self.emit_ldx_imm(Immediate::new(self.constant_u16(left).unwrap()).high());
            self.emit_jsr_runtime_helper(helper_slot, helper, left.span);
            self.emit_sta_slot_byte(slot, 0);

            if slot.size > 1 {
                self.emit_txa();
                self.emit_sta_slot_byte(slot, 1);
            }

            return true;
        }

        let mut left_low_loaded = false;
        if let Some(left_slot) = materialized_left {
            if left_slot.size > 1 {
                self.emit_lda_slot_byte(left_slot, 1);
                self.emit_tax();
                self.emit_lda_slot_byte(left_slot, 0);
            } else {
                self.emit_lda_slot_byte(left_slot, 0);
                self.emit_ldx_imm(0);
            }
        } else if self.segment_storage && self.expr_size(left) == Some(1) {
            if !self.emit_load_simple_byte(left, 0) {
                return false;
            }
            left_low_loaded = true;
            self.emit_ldx_imm(0);
        } else {
            if !self.emit_load_simple_byte(left, 1) {
                return false;
            }
            self.emit_tax();
        }

        if materialized_left.is_none() && !left_low_loaded && !self.emit_load_simple_byte(left, 0) {
            return false;
        }
        self.emit_jsr_runtime_helper(helper_slot, helper, left.span);
        self.emit_sta_slot_byte(slot, 0);

        if slot.size > 1 {
            self.emit_txa();
            self.emit_sta_slot_byte(slot, 1);
        }

        true
    }

    pub(super) fn emit_runtime_right_indexed_word_to_afcur(&mut self, right: &Expr) -> bool {
        if !self.segment_storage {
            return false;
        }
        let Some(source) =
            self.dynamic_indexed_word_slot_with_pointer(right, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if source.size != 2 {
            return false;
        }

        self.emit_ldy_imm(1);
        self.emit_lda_slot_byte(source, 1);
        self.emit_sta_zero_page(runtime_zp::AFCUR.offset(1));
        self.emit_dey();
        self.emit_lda_slot_byte(source, 0);
        self.emit_sta_zero_page(runtime_zp::AFCUR);
        true
    }

    pub(super) fn emit_runtime_right_call_result_to_afcur(&mut self, right: &Expr) -> bool {
        if !self.segment_storage {
            return false;
        }
        let ExprKind::Call { callee, args } = &right.kind else {
            return false;
        };
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };
        if return_slot.size != 2 || !self.emit_call(callee, args, right.span) {
            return false;
        }

        self.emit_lda_slot_byte(return_slot, 1);
        self.emit_sta_zero_page(runtime_zp::AFCUR.offset(1));
        self.emit_lda_slot_byte(return_slot, 0);
        self.emit_sta_zero_page(runtime_zp::AFCUR);
        true
    }

    pub(super) fn emit_jsr_runtime_helper(
        &mut self,
        helper_slot: RuntimeHelperSlot,
        target: RuntimeHelperTarget,
        span: Span,
    ) {
        debug_assert_runtime_helper_target_is_callable(&target);
        match target {
            RuntimeHelperTarget::Absolute(address) => self.emitter.emit_jsr_absolute(address),
            RuntimeHelperTarget::Label(label) => self.emitter.emit_jsr_label(label, span),
        }
        let effects = runtime_helper_effects(helper_slot);
        self.merge_current_callee_effects(effects);
        self.processor.invalidate_after_known_call(effects);
    }

    pub(super) fn emit_lsh_expr_to_slot(
        &mut self,
        left: &Expr,
        slot: StorageSlot,
        count: u16,
    ) -> bool {
        if slot.size == 1 {
            if !self.emit_load_simple_byte(left, 0) {
                if !self.segment_storage || self.expr_size(left) != Some(1) {
                    return false;
                }
                if !self.emit_expr_to_slot(left, slot) {
                    return false;
                }
                for _ in 0..count {
                    self.emit_lda_slot_byte(slot, 0);
                    self.emit_asl_a();
                    self.emit_sta_slot_byte(slot, 0);
                }
                return true;
            }
            for _ in 0..count {
                self.emit_asl_a();
            }
            self.emit_sta_slot_byte(slot, 0);
            return true;
        }

        if !self.emit_copy_expr_to_slot(left, slot) {
            return false;
        }
        for _ in 0..count {
            self.emit_lda_slot_byte(slot, 0);
            self.emit_asl_a();
            self.emit_sta_slot_byte(slot, 0);
            self.emit_lda_slot_byte(slot, 1);
            self.emit_rol_a();
            self.emit_sta_slot_byte(slot, 1);
        }
        true
    }

    pub(super) fn emit_rsh_expr_to_slot(
        &mut self,
        left: &Expr,
        slot: StorageSlot,
        count: u16,
    ) -> bool {
        if slot.size == 1 {
            if !self.emit_load_simple_byte(left, 0) {
                if !self.segment_storage || self.expr_size(left) != Some(1) {
                    return false;
                }
                if !self.emit_expr_to_slot(left, slot) {
                    return false;
                }
                for _ in 0..count {
                    self.emit_lda_slot_byte(slot, 0);
                    self.emit_lsr_a();
                    self.emit_sta_slot_byte(slot, 0);
                }
                return true;
            }
            for _ in 0..count {
                self.emit_lsr_a();
            }
            self.emit_sta_slot_byte(slot, 0);
            return true;
        }

        if !self.emit_copy_expr_to_slot(left, slot) {
            return false;
        }
        for _ in 0..count {
            self.emit_lda_slot_byte(slot, 1);
            self.emit_lsr_a();
            self.emit_sta_slot_byte(slot, 1);
            self.emit_lda_slot_byte(slot, 0);
            self.emit_ror_a();
            self.emit_sta_slot_byte(slot, 0);
        }
        true
    }

    pub(super) fn emit_binary_expr_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        let ExprKind::Binary { op, left, right } = &expr.kind else {
            return false;
        };
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && self.emit_address_add_expr_to_slot(expr, left, right, slot)
        {
            return true;
        }
        if self.segment_storage
            && matches!(slot.size, 1 | 2)
            && self.emit_runtime_mul_add_sub_to_slot(*op, left, right, slot)
        {
            return true;
        }
        if matches!(op, BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod) {
            return self.emit_runtime_binary_expr_to_slot(*op, left, right, slot);
        }
        if let Some(expr_size) = self.expr_size(expr)
            && expr_size < slot.size
        {
            if self.segment_storage
                && expr_size == 1
                && slot.size == 2
                && self.emit_byte_runtime_mul_add_sub_to_word_slot(*op, left, right, slot)
            {
                return true;
            }
            let narrow_slot = slot.with_size(expr_size);
            if !self.emit_binary_expr_to_slot(expr, narrow_slot) {
                return false;
            }
            for byte_index in expr_size..slot.size {
                self.emit_lda_imm(0);
                self.emit_sta_slot_byte(slot, byte_index);
            }
            return true;
        }
        if self.segment_storage {
            match op {
                BinaryOp::Add if self.expr_is_effective_zero(right) => {
                    return self.emit_expr_to_slot(left, slot);
                }
                BinaryOp::Add if self.expr_is_effective_zero(left) => {
                    return self.emit_expr_to_slot(right, slot);
                }
                BinaryOp::Sub if self.expr_is_effective_zero(right) => {
                    return self.emit_expr_to_slot(left, slot);
                }
                BinaryOp::Or | BinaryOp::Xor if self.expr_is_effective_zero(right) => {
                    return self.emit_expr_to_slot(left, slot);
                }
                BinaryOp::Or | BinaryOp::Xor if self.expr_is_effective_zero(left) => {
                    return self.emit_expr_to_slot(right, slot);
                }
                _ => {}
            }
        }
        if matches!(op, BinaryOp::Lsh | BinaryOp::Rsh) {
            return self.emit_shift_expr_to_slot(*op, left, right, slot);
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && self.constant_u16(right) == Some(1)
            && self.direct_scalar_slot(left) == Some(slot)
        {
            self.emit_increment_slot(slot, 1);
            return true;
        }
        if self.segment_storage
            && self.emit_byte_lvalue_constant_arithmetic_to_slot(*op, left, right, slot)
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && let Some(value) = self.constant_u16(right)
            && self.emit_add_constant_indexed_word_to_slot(left, value, slot)
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && let Some(value) = self.constant_u16(left)
            && self.emit_add_constant_indexed_word_to_slot(right, value, slot)
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && let Some(value) = self.constant_u16(left)
            && self.emit_add_constant_to_byte_constant_shift_to_slot(right, value, slot)
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && let Some(value) = self.constant_u16(right)
            && self.emit_add_constant_to_byte_constant_shift_to_slot(left, value, slot)
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && self.emit_add_indexed_word_expr_to_slot_with_pointer(
                right,
                left,
                slot,
                runtime_zp::ARRAY_ADDR,
            )
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && self.emit_add_indexed_word_expr_to_slot_with_pointer(
                left,
                right,
                slot,
                runtime_zp::ARRAY_ADDR,
            )
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && self.direct_scalar_slot(left) == Some(slot)
            && Self::arithmetic_operand_needs_materialization(right)
            && self.emit_add_lvalue_word_to_slot(right, slot)
        {
            return true;
        }
        if self.segment_storage
            && *op == BinaryOp::Add
            && slot.size == 2
            && self.direct_scalar_slot(right) == Some(slot)
            && Self::arithmetic_operand_needs_materialization(left)
            && self.emit_add_lvalue_word_to_slot(left, slot)
        {
            return true;
        }
        if self.segment_storage
            && BinaryScalarOp::from_binary(*op).is_some()
            && slot.size == 1
            && Self::arithmetic_operand_needs_materialization(left)
            && Self::arithmetic_operand_needs_materialization(right)
            && self.emit_binary_lvalue_lvalue_byte_to_slot(*op, left, right, slot)
        {
            return true;
        }
        if self.segment_storage
            && let Some(arithmetic_op) = BinaryArithmeticOp::from_binary(*op)
            && slot.size == 2
            && Self::arithmetic_operand_needs_materialization(left)
            && Self::arithmetic_operand_needs_materialization(right)
            && self.emit_same_record_pointer_word_fields_to_slot(
                arithmetic_op,
                left,
                right,
                slot,
                runtime_zp::ARRAY_ADDR,
            )
        {
            return true;
        }
        if self.segment_storage
            && let Some(scalar_op) = BinaryScalarOp::from_binary(*op)
            && Self::arithmetic_operand_needs_materialization(left)
            && slot.size <= 2
        {
            let bitwise = scalar_op.is_bitwise();
            if self.arithmetic_operand_needs_codegen_temp(right) {
                if let BinaryScalarOp::Bitwise(bitwise_op) = scalar_op
                    && slot.size == 2
                    && self.emit_same_record_pointer_word_bitwise_fields_to_slot(
                        bitwise_op,
                        left,
                        right,
                        slot,
                        runtime_zp::ARRAY_ADDR,
                    )
                {
                    return true;
                }
                if *op == BinaryOp::Add
                    && Self::expr_uses_runtime_multiply(right)
                    && self.emit_add_left_then_runtime_multiply_to_slot(left, right, slot)
                {
                    return true;
                }
                let right_slot = StorageSlot::zero_page(runtime_zp::ADDR.address(), slot.size);
                if *op == BinaryOp::Add
                    && (expr_contains_routine_call(left, &self.routines)
                        || Self::expr_uses_runtime_arithmetic_helper(left))
                {
                    let left_slot =
                        StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), slot.size);
                    if !self.emit_expr_to_slot(left, left_slot) {
                        return false;
                    }
                    for byte_index in (0..left_slot.size).rev() {
                        self.emit_lda_slot_byte(left_slot, byte_index);
                        self.emitter.emit_pha();
                    }
                    let right_emitted = if bitwise {
                        self.emit_bitwise_operand_to_slot(right, right_slot)
                    } else {
                        self.emit_expr_to_slot(right, right_slot)
                    };
                    if !right_emitted {
                        return false;
                    }
                    for byte_index in 0..left_slot.size {
                        self.emit_pla();
                        self.emit_sta_slot_byte(left_slot, byte_index);
                    }
                    return self.emit_binary_slot_slot_to_slot(*op, left_slot, right_slot, slot);
                }
                if self.profile.enables_modern_optimizations()
                    && expr_contains_routine_call(left, &self.routines)
                    && expr_contains_routine_call(right, &self.routines)
                {
                    let left_slot =
                        StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), slot.size);
                    let left_emitted = if bitwise {
                        self.emit_bitwise_operand_to_slot(left, left_slot)
                    } else {
                        self.emit_expr_to_slot(left, left_slot)
                    };
                    if !left_emitted {
                        return false;
                    }
                    let right_emitted = if bitwise {
                        self.emit_bitwise_operand_to_slot(right, right_slot)
                    } else {
                        self.emit_expr_to_slot(right, right_slot)
                    };
                    if !right_emitted {
                        return false;
                    }
                    return self.emit_binary_slot_slot_to_slot(*op, left_slot, right_slot, slot);
                }
                let right_emitted = if bitwise {
                    self.emit_bitwise_operand_to_slot(right, right_slot)
                } else {
                    self.emit_expr_to_slot(right, right_slot)
                };
                if !right_emitted {
                    return false;
                }
                let left_slot =
                    StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), slot.size);
                let left_emitted = if bitwise {
                    self.emit_bitwise_operand_to_slot(left, left_slot)
                } else {
                    self.emit_expr_to_slot(left, left_slot)
                };
                if !left_emitted {
                    return false;
                }
                return self.emit_binary_slot_slot_to_slot(*op, left_slot, right_slot, slot);
            }
            let temp = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), slot.size);
            let emitted = if bitwise {
                self.emit_bitwise_operand_to_slot(left, temp)
            } else {
                self.emit_expr_to_slot(left, temp)
            };
            if !emitted {
                return false;
            }
            return self.emit_binary_slot_expr_to_slot(*op, temp, right, slot);
        }
        if self.segment_storage
            && *op == BinaryOp::Sub
            && slot.size <= 2
            && !Self::arithmetic_operand_needs_materialization(left)
            && Self::arithmetic_operand_needs_materialization(right)
        {
            if let Some(right_slot) =
                self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ARRAY_ADDR)
            {
                return self.emit_binary_expr_slot_to_slot(*op, left, right_slot, slot);
            }
            if self.emit_sub_materialized_rhs_to_slot(left, right, slot) {
                return true;
            }
        }
        if self.segment_storage
            && BinaryScalarOp::from_binary(*op)
                .is_some_and(BinaryScalarOp::allows_reversed_materialized_rhs)
            && Self::arithmetic_operand_needs_materialization(right)
            && slot.size <= 2
        {
            let bitwise = BinaryBitwiseOp::from_binary(*op).is_some();
            let temp = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), slot.size);
            let emitted = if bitwise {
                self.emit_bitwise_operand_to_slot(right, temp)
            } else {
                self.emit_expr_to_slot(right, temp)
            };
            if !emitted {
                return false;
            }
            return self.emit_binary_slot_expr_to_slot(*op, temp, left, slot);
        }

        self.emit_binary_bytes_to_slot(slot, |generator, byte_index, set_carry| {
            generator.emit_binary_expr_byte(*op, left, right, byte_index, set_carry)
        })
    }

    pub(super) fn emit_sub_materialized_rhs_to_slot(
        &mut self,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        if target.size > 2
            || Self::arithmetic_operand_needs_materialization(left)
            || !Self::arithmetic_operand_needs_materialization(right)
        {
            return false;
        }

        let Some(temp) = self.materialized_rhs_sub_temp_slot(target) else {
            return false;
        };
        if !self.emit_expr_to_slot(right, temp) {
            return false;
        }
        self.emit_binary_expr_slot_to_slot(BinaryOp::Sub, left, temp, target)
    }

    fn materialized_rhs_sub_temp_slot(&self, target: StorageSlot) -> Option<StorageSlot> {
        [
            runtime_zp::ARRAY_ADDR,
            runtime_zp::ELEMENT_ADDR,
            runtime_zp::ADDR,
        ]
        .into_iter()
        .find(|zero_page| !slot_overlaps_zero_page(target, *zero_page, target.size))
        .map(|zero_page| StorageSlot::zero_page(zero_page.address(), target.size))
    }

    pub(super) fn emit_address_add_expr_to_slot(
        &mut self,
        expr: &Expr,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        let Some(lowering) = self.address_add_lowering(expr, left, right, target) else {
            return false;
        };
        match lowering {
            AddressAddLowering::PointerPlusOwnByteDeref { pointer, offset } => {
                self.emit_pointer_plus_own_byte_deref_to_slot(pointer, offset, target)
            }
            AddressAddLowering::ArrayPointerPlusByte { array, addend } => {
                self.emit_array_pointer_plus_byte_to_slot(array, addend, target)
            }
        }
    }

    fn address_add_lowering<'a>(
        &self,
        expr: &'a Expr,
        left: &'a Expr,
        right: &'a Expr,
        target: StorageSlot,
    ) -> Option<AddressAddLowering<'a>> {
        if target.size != 2 {
            return None;
        }
        if let Some((pointer, offset)) = self.pointer_plus_own_byte_deref_parts(expr, 0)
            && pointer.size == 2
            && pointer.pointee_size == Some(1)
        {
            return Some(AddressAddLowering::PointerPlusOwnByteDeref { pointer, offset });
        }
        self.array_pointer_plus_byte_lowering(left, right)
            .or_else(|| self.array_pointer_plus_byte_lowering(right, left))
    }

    fn array_pointer_plus_byte_lowering<'a>(
        &self,
        array: &'a Expr,
        addend: &'a Expr,
    ) -> Option<AddressAddLowering<'a>> {
        let array = self.array_pointer_value_slot(array)?;
        (self.expr_size(addend) == Some(1))
            .then_some(AddressAddLowering::ArrayPointerPlusByte { array, addend })
    }

    pub(super) fn emit_pointer_plus_own_byte_deref_to_slot(
        &mut self,
        pointer: StorageSlot,
        offset: u16,
        target: StorageSlot,
    ) -> bool {
        if !self.emit_pointer_slot_to_addr(pointer, runtime_zp::ARRAY_ADDR) {
            return false;
        }
        let deref = StorageSlot::indirect_indexed_y(runtime_zp::ARRAY_ADDR, 1);
        let target_is_sum_temp = target.space == AddressSpace::ZeroPage
            && target.address == u16::from(runtime_zp::ELEMENT_ADDR.address());
        let sum = if offset == 0 || target_is_sum_temp {
            target
        } else {
            StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2)
        };

        self.emit_lda_slot_byte_value_only(pointer, 0);
        self.ensure_y_imm(0);
        self.emit_clc();
        self.emit_adc_slot_byte(deref, 0);
        self.emit_sta_slot_byte(sum, 0);
        self.emit_lda_slot_byte_value_only(pointer, 1);
        self.emit_adc_imm(0);
        self.emit_sta_slot_byte(sum, 1);

        if offset != 0 {
            let immediate = Immediate::new(offset);
            self.emit_clc();
            self.emit_lda_slot_byte(sum, 0);
            self.emit_adc_immediate(immediate, 0);
            self.emit_sta_slot_byte(target, 0);
            self.emit_lda_slot_byte(sum, 1);
            self.emit_adc_immediate(immediate, 1);
            self.emit_sta_slot_byte(target, 1);
        }
        true
    }

    pub(super) fn pointer_plus_own_byte_deref_parts(
        &self,
        expr: &Expr,
        offset: u16,
    ) -> Option<(StorageSlot, u16)> {
        let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = &expr.kind
        else {
            return None;
        };

        if let Some(value) = self.constant_u16(right) {
            return self.pointer_plus_own_byte_deref_parts(left, offset.wrapping_add(value));
        }
        if let Some(value) = self.constant_u16(left) {
            return self.pointer_plus_own_byte_deref_parts(right, offset.wrapping_add(value));
        }

        self.own_byte_deref_pointer_slot(left, right)
            .or_else(|| self.own_byte_deref_pointer_slot(right, left))
            .map(|slot| (slot, offset))
    }

    pub(super) fn own_byte_deref_pointer_slot(
        &self,
        pointer_expr: &Expr,
        deref_expr: &Expr,
    ) -> Option<StorageSlot> {
        let ExprKind::Name(pointer_name) = &pointer_expr.kind else {
            return None;
        };
        let ExprKind::Unary {
            op: UnaryOp::Deref,
            expr,
        } = &deref_expr.kind
        else {
            return None;
        };
        let ExprKind::Name(deref_name) = &expr.kind else {
            return None;
        };
        if pointer_name != deref_name {
            return None;
        }
        let slot = self.lookup_slot(pointer_name)?;
        (slot.pointee_size == Some(1)).then_some(slot)
    }

    pub(super) fn emit_add_left_then_runtime_multiply_to_slot(
        &mut self,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        if target.size > 2 {
            return false;
        }
        let preserve_left_on_stack = !Self::runtime_multiply_has_simple_operands(right);
        let (left_slot, right_slot) = if Self::expr_contains_indirect_lvalue(left) {
            (
                StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), target.size),
                StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), target.size),
            )
        } else {
            (
                StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), target.size),
                StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), target.size),
            )
        };
        if !self.emit_expr_to_slot(left, left_slot) {
            return false;
        }
        if preserve_left_on_stack {
            for byte_index in (0..left_slot.size).rev() {
                self.emit_lda_slot_byte(left_slot, byte_index);
                self.emitter.emit_pha();
            }
        }
        if !self.emit_expr_to_slot(right, right_slot) {
            return false;
        }
        if preserve_left_on_stack {
            for byte_index in 0..left_slot.size {
                self.emit_pla();
                self.emit_sta_slot_byte(left_slot, byte_index);
            }
        }
        self.emit_binary_slot_slot_to_slot(BinaryOp::Add, left_slot, right_slot, target)
    }

    pub(super) fn emit_byte_lvalue_constant_arithmetic_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        if target.size != 1 {
            return false;
        }
        let Some(arithmetic_op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        let (source_expr, value) = match op {
            BinaryOp::Add => {
                if let Some(value) = self.constant_u16(right) {
                    (left, value)
                } else if let Some(value) = self.constant_u16(left) {
                    (right, value)
                } else {
                    return false;
                }
            }
            BinaryOp::Sub => {
                let Some(value) = self.constant_u16(right) else {
                    return false;
                };
                (left, value)
            }
            _ => return false,
        };
        let Some(source) =
            self.reusable_lvalue_slot_with_pointer_or_direct(source_expr, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if source.size != 1 {
            return false;
        }

        self.emit_arithmetic_carry_setup(arithmetic_op);
        self.emit_lda_slot_byte_value_only(source, 0);
        self.emit_arithmetic_immediate(arithmetic_op, Immediate::new(value), 0);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_runtime_mul_add_sub_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        if op != BinaryOp::Sub || target.size > 2 {
            return false;
        }
        let Some(subtrahend) = self.constant_u16(right) else {
            return false;
        };
        let ExprKind::Binary {
            op: BinaryOp::Add,
            left: add_left,
            right: add_right,
        } = &left.kind
        else {
            return false;
        };
        let Some((mul_left, mul_right, addend)) =
            Self::runtime_mul_plus_byte_parts(add_left, add_right, self.expr_size(add_right))
                .or_else(|| {
                    Self::runtime_mul_plus_byte_parts(add_right, add_left, self.expr_size(add_left))
                })
        else {
            return false;
        };

        let mul_result = StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), 2);
        if !self.emit_runtime_binary_expr_to_slot(BinaryOp::Mul, mul_left, mul_right, mul_result) {
            return false;
        }

        let sum = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 2);
        self.emit_clc();
        self.emit_lda_slot_byte(mul_result, 0);
        if !self.emit_add_simple_byte(addend, 0) {
            return false;
        }
        self.emit_sta_slot_byte(sum, 0);
        self.emit_lda_slot_byte(mul_result, 1);
        self.emit_adc_imm(0);
        self.emit_sta_slot_byte(sum, 1);

        let immediate = Immediate::new(subtrahend);
        self.emit_sec();
        self.emit_lda_slot_byte(sum, 0);
        self.emit_sbc_immediate(immediate, 0);
        self.emit_sta_slot_byte(target, 0);
        if target.size > 1 {
            self.emit_lda_slot_byte(sum, 1);
            self.emit_sbc_immediate(immediate, 1);
            self.emit_sta_slot_byte(target, 1);
        }
        true
    }

    pub(super) fn emit_byte_runtime_mul_add_sub_to_word_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        let Some(arithmetic_op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        let Some((mul_left, mul_right, addend)) =
            self.byte_runtime_mul_add_sub_parts(op, left, right)
        else {
            return false;
        };

        let mul_result = StorageSlot::zero_page(runtime_zp::ARRAY_ADDR.address(), 2);
        if !self.emit_runtime_binary_expr_to_slot(BinaryOp::Mul, mul_left, mul_right, mul_result) {
            return false;
        }

        self.emit_arithmetic_carry_setup(arithmetic_op);
        self.emit_lda_slot_byte(mul_result, 0);
        if !self.emit_arithmetic_simple_byte(arithmetic_op, addend, 0) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_slot_byte(mul_result, 1);
        self.emit_arithmetic_zero(arithmetic_op);
        self.emit_sta_slot_byte(target, 1);
        true
    }

    fn byte_runtime_mul_add_sub_parts<'a>(
        &self,
        op: BinaryOp,
        left: &'a Expr,
        right: &'a Expr,
    ) -> Option<(&'a Expr, &'a Expr, &'a Expr)> {
        match op {
            BinaryOp::Add => self
                .byte_runtime_mul_and_byte_parts(left, right)
                .or_else(|| self.byte_runtime_mul_and_byte_parts(right, left)),
            BinaryOp::Sub => self.byte_runtime_mul_and_byte_parts(left, right),
            _ => None,
        }
    }

    fn byte_runtime_mul_and_byte_parts<'a>(
        &self,
        maybe_mul: &'a Expr,
        maybe_byte: &'a Expr,
    ) -> Option<(&'a Expr, &'a Expr, &'a Expr)> {
        if self.expr_size(maybe_byte) != Some(1) {
            return None;
        }
        if self.constant_u16(maybe_byte).is_none()
            && Self::arithmetic_operand_needs_materialization(maybe_byte)
        {
            return None;
        }
        let ExprKind::Binary {
            op: BinaryOp::Mul,
            left,
            right,
        } = &maybe_mul.kind
        else {
            return None;
        };
        if self.expr_size(left) != Some(1) || self.expr_size(right) != Some(1) {
            return None;
        }
        Some((left, right, maybe_byte))
    }

    pub(super) fn expr_uses_runtime_multiply(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Binary {
                op: BinaryOp::Mul,
                ..
            }
        )
    }

    fn expr_uses_runtime_arithmetic_helper(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                matches!(op, BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod)
                    || Self::expr_uses_runtime_arithmetic_helper(left)
                    || Self::expr_uses_runtime_arithmetic_helper(right)
            }
            ExprKind::Cast { expr, .. } | ExprKind::Unary { expr, .. } => {
                Self::expr_uses_runtime_arithmetic_helper(expr)
            }
            ExprKind::Index { base, index } => {
                Self::expr_uses_runtime_arithmetic_helper(base)
                    || Self::expr_uses_runtime_arithmetic_helper(index)
            }
            ExprKind::Call { callee, args } => {
                Self::expr_uses_runtime_arithmetic_helper(callee)
                    || args.iter().any(Self::expr_uses_runtime_arithmetic_helper)
            }
            ExprKind::Field { base, .. } => Self::expr_uses_runtime_arithmetic_helper(base),
            ExprKind::Missing
            | ExprKind::Raw
            | ExprKind::CurrentLocation
            | ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Char(_)
            | ExprKind::Name(_) => false,
        }
    }

    fn runtime_multiply_has_simple_operands(expr: &Expr) -> bool {
        let ExprKind::Binary {
            op: BinaryOp::Mul,
            left,
            right,
        } = &expr.kind
        else {
            return false;
        };
        !Self::arithmetic_operand_needs_materialization(left)
            && !Self::arithmetic_operand_needs_materialization(right)
    }

    pub(super) fn runtime_mul_plus_byte_parts<'a>(
        maybe_mul: &'a Expr,
        maybe_addend: &'a Expr,
        addend_size: Option<u16>,
    ) -> Option<(&'a Expr, &'a Expr, &'a Expr)> {
        if addend_size != Some(1) {
            return None;
        }
        let ExprKind::Binary {
            op: BinaryOp::Mul,
            left,
            right,
        } = &maybe_mul.kind
        else {
            return None;
        };
        Some((left, right, maybe_addend))
    }

    pub(super) fn emit_add_constant_to_byte_constant_shift_to_slot(
        &mut self,
        shifted: &Expr,
        value: u16,
        target: StorageSlot,
    ) -> bool {
        let Some((op, left, count)) = self.byte_constant_shift_parts(shifted) else {
            return false;
        };
        if count >= 8 {
            return false;
        }

        if !self.emit_byte_constant_shift_to_acc(op, left, count) {
            return false;
        }
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);
        let immediate = Immediate::new(value);
        self.emit_clc();
        self.emit_lda_immediate(immediate, 0);
        self.emit_adc_zero_page(runtime_zp::ARRAY_ADDR);
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_immediate(immediate, 1);
        self.emit_adc_imm(0);
        self.emit_sta_slot_byte(target, 1);
        true
    }

    pub(super) fn emit_array_pointer_plus_byte_to_slot(
        &mut self,
        array_slot: StorageSlot,
        addend: &Expr,
        target: StorageSlot,
    ) -> bool {
        self.emit_clc();
        self.emit_load_array_pointer_value_slot_byte_value_only(array_slot, 0);
        if !self.emit_add_simple_byte(addend, 0) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);
        self.emit_load_array_pointer_value_slot_byte_value_only(array_slot, 1);
        self.emit_adc_imm(0);
        self.emit_sta_slot_byte(target, 1);
        true
    }

    pub(super) fn emit_add_lvalue_word_to_slot(
        &mut self,
        value: &Expr,
        target: StorageSlot,
    ) -> bool {
        let Some(source) = self.reusable_lvalue_slot_with_pointer(value, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if source.size != 2 || target.size != 2 {
            return false;
        }

        self.emit_clc();
        self.emit_lda_slot_byte(target, 0);
        self.emit_adc_slot_byte(source, 0);
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_slot_byte(target, 1);
        self.emit_adc_slot_byte(source, 1);
        self.emit_sta_slot_byte(target, 1);
        true
    }

    pub(super) fn emit_binary_lvalue_lvalue_byte_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
    ) -> bool {
        if self.emit_same_record_pointer_byte_fields_to_slot(
            op,
            left,
            right,
            target,
            runtime_zp::ARRAY_ADDR,
        ) {
            return true;
        }
        let Some(left_slot) = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        let Some(right_slot) =
            self.reusable_lvalue_slot_with_pointer(right, runtime_zp::ELEMENT_ADDR)
        else {
            return false;
        };
        if left_slot.size != 1 || right_slot.size != 1 {
            return false;
        }

        if !self.emit_binary_scalar_byte(
            op,
            BinaryByteSource::Slot(left_slot),
            BinaryByteOperand::Slot(right_slot),
            0,
            true,
            BinaryCarryTiming::BeforeLeft,
        ) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);
        true
    }

    fn emit_same_record_pointer_byte_fields_to_slot(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
        pointer: ZeroPage,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        let Some(op) = BinaryScalarOp::from_binary(op) else {
            return false;
        };
        if target.size != 1 || slot_overlaps_zero_page(target, pointer, 2) {
            return false;
        }
        let Some((base, left_field, right_field)) =
            self.same_record_pointer_byte_fields(left, right)
        else {
            return false;
        };
        if !self.emit_pointer_slot_to_addr(base, pointer) {
            return false;
        }

        let left_slot = StorageSlot::indirect_indexed_y(pointer, 1)
            .offset_bytes(left_field.offset)
            .signed(left_field.signed);
        let right_slot = StorageSlot::indirect_indexed_y(pointer, 1)
            .offset_bytes(right_field.offset)
            .signed(right_field.signed);
        self.emit_binary_scalar_byte(
            match op {
                BinaryScalarOp::Arithmetic(BinaryArithmeticOp::Add) => BinaryOp::Add,
                BinaryScalarOp::Arithmetic(BinaryArithmeticOp::Sub) => BinaryOp::Sub,
                BinaryScalarOp::Bitwise(BinaryBitwiseOp::And) => BinaryOp::And,
                BinaryScalarOp::Bitwise(BinaryBitwiseOp::Or) => BinaryOp::Or,
                BinaryScalarOp::Bitwise(BinaryBitwiseOp::Xor) => BinaryOp::Xor,
            },
            BinaryByteSource::Slot(left_slot),
            BinaryByteOperand::Slot(right_slot),
            0,
            true,
            BinaryCarryTiming::BeforeLeft,
        );
        self.emit_sta_slot_byte(target, 0);
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressReused,
            4,
            Some(left.span),
            "reused record pointer base for byte field expression",
        );
        true
    }

    fn same_record_pointer_byte_fields(
        &self,
        left: &Expr,
        right: &Expr,
    ) -> Option<(StorageSlot, RecordField, RecordField)> {
        self.same_record_pointer_fields_with_size(left, right, 1)
    }

    fn record_field_parts(expr: &Expr) -> Option<(&Expr, &str)> {
        let ExprKind::Field { base, field } = &expr.kind else {
            return None;
        };
        Some((base.as_ref(), field.as_str()))
    }

    pub(super) fn emit_add_constant_indexed_word_to_slot(
        &mut self,
        indexed: &Expr,
        value: u16,
        target: StorageSlot,
    ) -> bool {
        let Some(source) = self.dynamic_indexed_word_slot(indexed) else {
            return false;
        };
        let immediate = Immediate::new(value);
        self.emit_clc();
        self.emit_lda_slot_byte(source, 0);
        self.emit_adc_immediate(immediate, 0);
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_slot_byte(source, 1);
        self.emit_adc_immediate(immediate, 1);
        self.emit_sta_slot_byte(target, 1);
        true
    }

    pub(super) fn dynamic_indexed_word_slot(&mut self, expr: &Expr) -> Option<StorageSlot> {
        let (base, index) = match &expr.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return None,
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let array = self.lookup_slot(name)?;
        if !matches!(
            array.array?,
            ArrayStorage::Pointer | ArrayStorage::Descriptor
        ) || array.pointee_size.is_some()
            || array.size != 2
            || self.constant_u16(index).is_some()
        {
            return None;
        }
        self.index_slot(base, index)
    }

    pub(super) fn emit_add_constant_indexed_word_to_slot_with_pointer(
        &mut self,
        indexed: &Expr,
        value: u16,
        target: StorageSlot,
        pointer: ZeroPage,
    ) -> bool {
        let Some(source) = self.dynamic_indexed_word_slot_with_pointer(indexed, pointer) else {
            return false;
        };
        let immediate = Immediate::new(value);
        self.emit_clc();
        self.emit_lda_slot_byte(source, 0);
        self.emit_adc_immediate(immediate, 0);
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_slot_byte(source, 1);
        self.emit_adc_immediate(immediate, 1);
        self.emit_sta_slot_byte(target, 1);
        true
    }

    pub(super) fn emit_add_indexed_word_expr_to_slot_with_pointer(
        &mut self,
        indexed: &Expr,
        addend: &Expr,
        target: StorageSlot,
        pointer: ZeroPage,
    ) -> bool {
        if target.size != 2 {
            return false;
        }
        if let Some(addend) = self.direct_scalar_slot(addend)
            && addend.size == 2
            && self.emit_add_effective_indexed_word_direct_to_slot(indexed, addend, target, pointer)
        {
            return true;
        }
        if Self::arithmetic_operand_needs_materialization(addend) {
            return false;
        }
        let Some(source) = self.reusable_lvalue_slot_with_pointer(indexed, pointer) else {
            return false;
        };
        if source.size != 2 {
            return false;
        }
        self.emit_clc();
        self.emit_lda_slot_byte(source, 0);
        if !self.emit_add_simple_byte(addend, 0) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_slot_byte(source, 1);
        if !self.emit_add_simple_byte(addend, 1) {
            return false;
        }
        self.emit_sta_slot_byte(target, 1);
        true
    }

    fn emit_add_effective_indexed_word_direct_to_slot(
        &mut self,
        indexed: &Expr,
        addend: StorageSlot,
        target: StorageSlot,
        pointer: ZeroPage,
    ) -> bool {
        if slot_overlaps_zero_page(addend, pointer, 2)
            || slot_overlaps_zero_page(addend, runtime_zp::ARGS, 1)
            || slot_overlaps_zero_page(target, runtime_zp::ARGS, 1)
        {
            return false;
        }
        let Some(address) = self.byte_index_effective_address(indexed, pointer) else {
            return false;
        };
        if address.element_size != 2 || addend.size != 2 || target.size != 2 {
            return false;
        }
        if !self.emit_effective_address_pointer_and_y(address, 0) {
            return false;
        }

        self.emit_clc();
        self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(pointer));
        self.emit_adc_slot_byte(addend, 0);
        self.emit_sta_zero_page(runtime_zp::ARGS);
        self.emit_iny();
        self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(pointer));
        self.emit_adc_slot_byte(addend, 1);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_zero_page(runtime_zp::ARGS);
        self.emit_sta_slot_byte(target, 0);
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressLowered,
            3,
            Some(indexed.span),
            "used byte-indexed word effective address directly in arithmetic",
        );
        true
    }

    pub(super) fn dynamic_indexed_word_slot_with_pointer(
        &mut self,
        expr: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let (base, index) = match &expr.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return None,
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let array = self.lookup_slot(name)?;
        if !matches!(
            array.array?,
            ArrayStorage::Pointer | ArrayStorage::Descriptor
        ) || array.pointee_size.is_some()
            || array.size != 2
            || self.constant_u16(index).is_some()
        {
            return None;
        }
        if pointer == runtime_zp::ARRAY_ADDR
            && let Some(pointer) = self.emit_compatible_word_array_expr_index_address(array, index)
        {
            return Some(StorageSlot::indirect_indexed_y(pointer, array.size).signed(array.signed));
        }
        if !self.emit_array_base_plus_scaled_byte_index_to_pointer(array, index, pointer) {
            return None;
        }
        Some(StorageSlot::indirect_indexed_y(pointer, array.size).signed(array.signed))
    }

    fn arithmetic_operand_needs_codegen_temp(&self, expr: &Expr) -> bool {
        self.constant_u16(expr).is_none() && Self::arithmetic_operand_needs_materialization(expr)
    }

    pub(super) fn arithmetic_operand_needs_materialization(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Call { .. } | ExprKind::Index { .. } | ExprKind::Field { .. } => true,
            ExprKind::Unary {
                op: UnaryOp::Deref | UnaryOp::Neg,
                ..
            } => true,
            ExprKind::Unary { expr, .. } => Self::arithmetic_operand_needs_materialization(expr),
            ExprKind::Binary {
                op:
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::Lsh
                    | BinaryOp::Rsh
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod,
                ..
            } => true,
            _ => false,
        }
    }

    pub(super) fn emit_indirect_self_byte_arithmetic_assignment(
        &mut self,
        target_expr: &Expr,
        value: &Expr,
        target: StorageSlot,
    ) -> bool {
        if !self.segment_storage
            || target.space != AddressSpace::IndirectIndexedY
            || target.size != 1
        {
            return false;
        }
        let ExprKind::Binary { op, left, right } = &value.kind else {
            return false;
        };
        let (operand, constant) = if Self::same_lvalue_expr(target_expr, left) {
            (right.as_ref(), self.constant_u16(right))
        } else if *op == BinaryOp::Add && Self::same_lvalue_expr(target_expr, right) {
            (left.as_ref(), self.constant_u16(left))
        } else {
            return false;
        };
        let Some(value) = constant else {
            return false;
        };
        let Some(op) = BinaryArithmeticOp::from_binary(*op) else {
            return false;
        };
        if Self::arithmetic_operand_needs_materialization(operand) {
            return false;
        }

        let source_pointer = if target.zero_page_byte(0) == runtime_zp::ARRAY_ADDR {
            runtime_zp::ELEMENT_ADDR
        } else {
            runtime_zp::ARRAY_ADDR
        };
        self.emit_lda_zero_page(target.zero_page_byte(0));
        self.emit_sta_zero_page(source_pointer);
        self.emit_lda_zero_page(target.zero_page_byte(0).offset(1));
        self.emit_sta_zero_page(source_pointer.offset(1));
        let source = StorageSlot::indirect_indexed_y(source_pointer, target.size);

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(source, 0);
        self.emit_arithmetic_immediate(op, Immediate::new(value), 0);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_indirect_byte_lvalue_arithmetic_to_slot(
        &mut self,
        op: BinaryOp,
        assignment_target_expr: &Expr,
        target_expr: &Expr,
        source_expr: &Expr,
        target: StorageSlot,
        source_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        if !Self::same_lvalue_expr(assignment_target_expr, target_expr) {
            return false;
        }
        if target.size != 1 || !Self::simple_pointer_deref_expr(source_expr) {
            return false;
        }
        let Some(source) = self.reusable_lvalue_slot_with_pointer(source_expr, source_pointer)
        else {
            return false;
        };
        if source.size != 1 || !Self::same_indirect_byte_target(target_expr, target) {
            return false;
        }

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(target, 0);
        self.emit_arithmetic_slot_byte(op, source, 0);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn same_indirect_byte_target(expr: &Expr, target: StorageSlot) -> bool {
        target.space == AddressSpace::IndirectIndexedY
            && target.size == 1
            && Self::simple_pointer_deref_expr(expr)
    }

    pub(super) fn simple_pointer_deref_expr(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } if matches!(&expr.kind, ExprKind::Name(_))
        )
    }

    pub(super) fn emit_indirect_byte_lvalue_simple_arithmetic_to_slot(
        &mut self,
        op: BinaryOp,
        source_expr: &Expr,
        addend_expr: &Expr,
        target: StorageSlot,
        source_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        if target.size != 1 {
            return false;
        }
        let Some(source) = self.reusable_lvalue_slot_with_pointer(source_expr, source_pointer)
        else {
            return false;
        };
        if source.size != 1 {
            return false;
        }

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(source, 0);
        if !self.emit_arithmetic_simple_byte(op, addend_expr, 0) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_indirect_byte_lvalue_constant_arithmetic_to_slot(
        &mut self,
        op: BinaryOp,
        source_expr: &Expr,
        constant_expr: &Expr,
        target: StorageSlot,
        source_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        let Some(value) = self.constant_u16(constant_expr) else {
            return false;
        };
        let Some(source) = self.reusable_lvalue_slot_with_pointer(source_expr, source_pointer)
        else {
            return false;
        };
        if source.size != 1 {
            return false;
        }

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(source, 0);
        self.emit_arithmetic_immediate(op, Immediate::new(value), 0);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_indirect_byte_lvalue_lvalue_bitwise_to_slot(
        &mut self,
        op: BinaryOp,
        left_expr: &Expr,
        right_expr: &Expr,
        target: StorageSlot,
        left_pointer: ZeroPage,
        right_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryBitwiseOp::from_binary(op) else {
            return false;
        };
        if target.size != 1 {
            return false;
        }
        if self.expr_size(left_expr) != Some(1) || self.expr_size(right_expr) != Some(1) {
            return false;
        }
        if !self.lvalue_can_be_prepared_or_direct(left_expr)
            || !self.lvalue_can_be_prepared_or_direct(right_expr)
        {
            return false;
        }
        let Some(left) = self.reusable_lvalue_slot_with_pointer_or_direct(left_expr, left_pointer)
        else {
            return false;
        };
        let Some(right) =
            self.reusable_lvalue_slot_with_pointer_or_direct(right_expr, right_pointer)
        else {
            return false;
        };
        if left.size != 1 || right.size != 1 {
            return false;
        }

        self.emit_lda_slot_byte(left, 0);
        self.emit_bitwise_slot_byte(op, right, 0);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_indirect_byte_lvalue_simple_bitwise_to_slot(
        &mut self,
        op: BinaryOp,
        source_expr: &Expr,
        operand_expr: &Expr,
        target: StorageSlot,
        source_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryBitwiseOp::from_binary(op) else {
            return false;
        };
        if target.size != 1 {
            return false;
        }
        let Some(source) = self.reusable_lvalue_slot_with_pointer(source_expr, source_pointer)
        else {
            return false;
        };
        if source.size != 1 {
            return false;
        }

        self.emit_lda_slot_byte(source, 0);
        if !self.emit_bitwise_simple_byte(op, operand_expr, 0) {
            return false;
        }
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_indirect_word_lvalue_lvalue_arithmetic_to_slot(
        &mut self,
        op: BinaryOp,
        left_expr: &Expr,
        right_expr: &Expr,
        target: StorageSlot,
        left_pointer: ZeroPage,
        right_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        if target.size != 2 {
            return false;
        }
        if self.emit_same_record_pointer_word_fields_to_slot(
            op,
            left_expr,
            right_expr,
            target,
            left_pointer,
        ) {
            return true;
        }
        if op == BinaryArithmeticOp::Add
            && let Some(right) = self.direct_scalar_slot(right_expr)
            && right.size == 2
            && self.emit_add_effective_indexed_word_direct_to_slot(
                left_expr,
                right,
                target,
                left_pointer,
            )
        {
            return true;
        }
        if self.expr_size(left_expr) != Some(2) || self.expr_size(right_expr) != Some(2) {
            return false;
        }
        if !self.lvalue_can_be_prepared_or_direct(left_expr)
            || !self.lvalue_can_be_prepared_or_direct(right_expr)
        {
            return false;
        };
        let Some(left) = self.reusable_lvalue_slot_with_pointer_or_direct(left_expr, left_pointer)
        else {
            return false;
        };
        let Some(right) =
            self.reusable_lvalue_slot_with_pointer_or_direct(right_expr, right_pointer)
        else {
            return false;
        };
        if left.size != 2 || right.size != 2 {
            return false;
        }

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(left, 0);
        self.emit_arithmetic_slot_byte(op, right, 0);
        self.emit_sta_zero_page(runtime_zp::ARGS);
        self.emit_lda_slot_byte(left, 1);
        self.emit_arithmetic_slot_byte(op, right, 1);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_zero_page(runtime_zp::ARGS);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    fn emit_same_record_pointer_word_fields_to_slot(
        &mut self,
        op: BinaryArithmeticOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
        pointer: ZeroPage,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        if target.size != 2
            || slot_overlaps_zero_page(target, pointer, 2)
            || slot_overlaps_zero_page(target, runtime_zp::ARGS, 1)
        {
            return false;
        }
        let Some((base, left_field, right_field)) =
            self.same_record_pointer_word_fields(left, right)
        else {
            return false;
        };
        if !self.emit_pointer_slot_to_addr(base, pointer) {
            return false;
        }

        let left_slot = StorageSlot::indirect_indexed_y(pointer, 2)
            .offset_bytes(left_field.offset)
            .signed(left_field.signed);
        let right_slot = StorageSlot::indirect_indexed_y(pointer, 2)
            .offset_bytes(right_field.offset)
            .signed(right_field.signed);
        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(left_slot, 0);
        self.emit_arithmetic_slot_byte(op, right_slot, 0);
        self.emit_sta_zero_page(runtime_zp::ARGS);
        self.emit_lda_slot_byte(left_slot, 1);
        self.emit_arithmetic_slot_byte(op, right_slot, 1);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_zero_page(runtime_zp::ARGS);
        self.emit_sta_slot_byte(target, 0);
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressReused,
            4,
            Some(left.span),
            "reused record pointer base for word field expression",
        );
        true
    }

    fn same_record_pointer_word_fields(
        &self,
        left: &Expr,
        right: &Expr,
    ) -> Option<(StorageSlot, RecordField, RecordField)> {
        self.same_record_pointer_fields_with_size(left, right, 2)
    }

    fn same_record_pointer_fields_with_size(
        &self,
        left: &Expr,
        right: &Expr,
        size: u16,
    ) -> Option<(StorageSlot, RecordField, RecordField)> {
        let (left_base, left_field) = Self::record_field_parts(left)?;
        let (right_base, right_field) = Self::record_field_parts(right)?;
        if left_base != right_base {
            return None;
        }
        let ExprKind::Name(name) = &left_base.kind else {
            return None;
        };
        let base = self.lookup_slot(name)?;
        let record = base.record?;
        base.pointee_size?;
        let left = self.record_layouts.field(record, left_field)?;
        let right = self.record_layouts.field(record, right_field)?;
        (left.size == size
            && right.size == size
            && record_field_fits_indirect_y(left)
            && record_field_fits_indirect_y(right))
        .then_some((base, left, right))
    }

    pub(super) fn emit_indirect_word_lvalue_lvalue_bitwise_to_slot(
        &mut self,
        op: BinaryOp,
        left_expr: &Expr,
        right_expr: &Expr,
        target: StorageSlot,
        left_pointer: ZeroPage,
        right_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryBitwiseOp::from_binary(op) else {
            return false;
        };
        if target.size != 2 {
            return false;
        }
        if self.emit_same_record_pointer_word_bitwise_fields_to_slot(
            op,
            left_expr,
            right_expr,
            target,
            left_pointer,
        ) {
            return true;
        }
        if self.expr_size(left_expr) != Some(2) || self.expr_size(right_expr) != Some(2) {
            return false;
        }
        if !self.lvalue_can_be_prepared_or_direct(left_expr)
            || !self.lvalue_can_be_prepared_or_direct(right_expr)
        {
            return false;
        };
        let Some(left) = self.reusable_lvalue_slot_with_pointer_or_direct(left_expr, left_pointer)
        else {
            return false;
        };
        let Some(right) =
            self.reusable_lvalue_slot_with_pointer_or_direct(right_expr, right_pointer)
        else {
            return false;
        };
        if left.size != 2 || right.size != 2 {
            return false;
        }

        self.emit_lda_slot_byte(left, 1);
        self.emit_bitwise_slot_byte(op, right, 1);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_slot_byte(left, 0);
        self.emit_bitwise_slot_byte(op, right, 0);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    fn emit_same_record_pointer_word_bitwise_fields_to_slot(
        &mut self,
        op: BinaryBitwiseOp,
        left: &Expr,
        right: &Expr,
        target: StorageSlot,
        pointer: ZeroPage,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        if target.size != 2 || slot_overlaps_zero_page(target, pointer, 2) {
            return false;
        }
        let Some((base, left_field, right_field)) =
            self.same_record_pointer_word_fields(left, right)
        else {
            return false;
        };
        if !self.emit_pointer_slot_to_addr(base, pointer) {
            return false;
        }

        let left_slot = StorageSlot::indirect_indexed_y(pointer, 2)
            .offset_bytes(left_field.offset)
            .signed(left_field.signed);
        let right_slot = StorageSlot::indirect_indexed_y(pointer, 2)
            .offset_bytes(right_field.offset)
            .signed(right_field.signed);
        self.emit_lda_slot_byte(left_slot, 1);
        self.emit_bitwise_slot_byte(op, right_slot, 1);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_slot_byte(left_slot, 0);
        self.emit_bitwise_slot_byte(op, right_slot, 0);
        self.emit_sta_slot_byte(target, 0);
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressReused,
            4,
            Some(left.span),
            "reused record pointer base for word bitwise field expression",
        );
        true
    }

    pub(super) fn emit_indirect_word_lvalue_byte_arithmetic_to_slot(
        &mut self,
        op: BinaryOp,
        left_expr: &Expr,
        right_expr: &Expr,
        target: StorageSlot,
        left_pointer: ZeroPage,
        right_pointer: ZeroPage,
    ) -> bool {
        let Some(op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        if target.size != 2 {
            return false;
        }
        if self.expr_size(left_expr) != Some(2) || self.expr_size(right_expr) != Some(1) {
            return false;
        }
        if self.prepared_pointer_fact(right_expr).is_none() {
            return false;
        }
        if !self.lvalue_can_be_prepared_or_direct(left_expr)
            || !self.lvalue_can_be_prepared_or_direct(right_expr)
        {
            return false;
        }
        if self.expr_signed(right_expr) {
            return false;
        }
        let Some(left) = self.reusable_lvalue_slot_with_pointer_or_direct(left_expr, left_pointer)
        else {
            return false;
        };
        let Some(right) =
            self.reusable_lvalue_slot_with_pointer_or_direct(right_expr, right_pointer)
        else {
            return false;
        };
        if left.size != 2 || right.size != 1 {
            return false;
        }

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(left, 0);
        self.emit_arithmetic_slot_byte(op, right, 0);
        self.emit_sta_zero_page(runtime_zp::ARGS);
        self.emit_lda_slot_byte(left, 1);
        self.emit_arithmetic_zero(op);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_zero_page(runtime_zp::ARGS);
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_indirect_byte_compound_lvalue_direct(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
    ) -> bool {
        let Some(op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        if !self.segment_storage {
            return false;
        }
        if !self.is_pointer_deref_expr(target) || !self.is_pointer_deref_expr(value) {
            return false;
        }
        let Some(target_slot) =
            self.pointer_deref_slot_with_pointer_expr(target, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if target_slot.space != AddressSpace::IndirectIndexedY || target_slot.size != 1 {
            return false;
        }
        let source_pointer = if target_slot.zero_page_byte(0) == runtime_zp::ELEMENT_ADDR {
            runtime_zp::ARRAY_ADDR
        } else {
            runtime_zp::ELEMENT_ADDR
        };
        let Some(source_slot) = self.pointer_deref_slot_with_pointer_expr(value, source_pointer)
        else {
            return false;
        };
        if source_slot.space != AddressSpace::IndirectIndexedY || source_slot.size != 1 {
            return false;
        }

        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(target_slot, 0);
        self.emit_arithmetic_slot_byte(op, source_slot, 0);
        self.emit_sta_slot_byte(target_slot, 0);
        true
    }

    pub(super) fn emit_compatible_indirect_word_compound_direct(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
    ) -> bool {
        let Some(arithmetic_op) = BinaryArithmeticOp::from_binary(op) else {
            return false;
        };
        if !self.segment_storage {
            return false;
        }
        let Some(target_slot) =
            self.reusable_lvalue_slot_with_pointer(target, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if target_slot.space != AddressSpace::IndirectIndexedY || target_slot.size != 2 {
            return false;
        }

        if let Some(value_slot) = self.direct_scalar_slot(value) {
            return self.emit_indirect_word_compound_slots(
                target_slot,
                arithmetic_op,
                value_slot,
                runtime_zp::ELEMENT_ADDR,
            );
        }
        let Some(value_slot) =
            self.reusable_lvalue_slot_with_pointer(value, runtime_zp::ELEMENT_ADDR)
        else {
            return false;
        };
        if value_slot.space != AddressSpace::IndirectIndexedY || value_slot.size < 2 {
            return false;
        }
        self.emit_indirect_word_compound_slots(
            target_slot,
            arithmetic_op,
            value_slot,
            runtime_zp::VALUE_TEMP,
        )
    }

    pub(super) fn emit_indirect_word_compound_slots(
        &mut self,
        target_slot: StorageSlot,
        op: BinaryArithmeticOp,
        value_slot: StorageSlot,
        temp: ZeroPage,
    ) -> bool {
        self.emit_arithmetic_carry_setup(op);
        self.emit_lda_slot_byte(target_slot, 0);
        self.emit_arithmetic_slot_byte(op, value_slot, 0);
        self.emit_sta_zero_page(temp);
        self.emit_lda_slot_byte(target_slot, 1);
        if value_slot.size > 1 {
            self.emit_arithmetic_slot_byte(op, value_slot, 1);
        } else {
            self.emit_arithmetic_zero(op);
        }
        self.emit_sta_slot_byte(target_slot, 1);
        self.emit_lda_zero_page(temp);
        self.emit_dey();
        self.emit_sta_slot_byte(target_slot, 0);
        true
    }

    pub(super) fn emit_compatible_compound_peephole(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage {
            return false;
        }
        let constant = self.constant_u16(value);
        let bitwise_op = BinaryBitwiseOp::from_binary(op);
        let can_try_bitwise =
            bitwise_op.is_some() && constant.is_some_and(|value| value <= u16::from(u8::MAX));
        let can_try_inline_indexed_bitwise =
            bitwise_op.is_some() && self.inline_byte_array_direct_index(value).is_some();
        let can_try_bitwise_chain =
            bitwise_op.is_some() && self.is_byte_compound_bitwise_chain(value);
        let can_try_increment = op == BinaryOp::Add && constant == Some(1);
        let can_try_decrement = op == BinaryOp::Sub && constant == Some(1);
        if !can_try_bitwise
            && !can_try_inline_indexed_bitwise
            && !can_try_bitwise_chain
            && !can_try_increment
            && !can_try_decrement
        {
            return false;
        }
        let Some(slot) = self.compatible_compound_target_slot(target) else {
            return false;
        };
        if let Some(bitwise_op) = bitwise_op
            && self.emit_compatible_byte_compound_bitwise_chain(slot, bitwise_op, value)
        {
            return true;
        }
        if let Some(value) = constant
            && slot.size == 1
            && let Some(bitwise_op) = bitwise_op
        {
            self.emit_lda_slot_byte(slot, 0);
            self.emit_bitwise_immediate(bitwise_op, Immediate::new(value), 0);
            self.emit_sta_slot_byte(slot, 0);
            return true;
        }
        if slot.size == 1
            && slot.space != AddressSpace::AbsoluteX
            && let Some(bitwise_op) = bitwise_op
            && let Some((array, index)) = self.inline_byte_array_direct_index(value)
        {
            self.emit_lda_slot_byte(slot, 0);
            self.emit_ldx_slot_byte(index, 0);
            let value_slot = StorageSlot::absolute_x(array.address, 1).signed(array.signed);
            self.emit_bitwise_slot_byte(bitwise_op, value_slot, 0);
            self.emit_sta_slot_byte(slot, 0);
            return true;
        }
        match op {
            BinaryOp::Add => self.emit_word_inc_peephole(slot) || self.emit_inc_slot_peephole(slot),
            BinaryOp::Sub => {
                if self.emit_dec_slot_peephole(slot) {
                    true
                } else if constant == Some(1) && slot.size == 2 && slot.array.is_none() {
                    self.emit_decrement_slot(slot, 1);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub(super) fn emit_compatible_byte_compound_bitwise_chain(
        &mut self,
        target: StorageSlot,
        op: BinaryBitwiseOp,
        value: &Expr,
    ) -> bool {
        if target.size != 1 {
            return false;
        }
        let ExprKind::Binary {
            op: next_op,
            left,
            right,
        } = &value.kind
        else {
            return false;
        };
        let Some(next_op) = BinaryBitwiseOp::from_binary(*next_op) else {
            return false;
        };
        if !self.is_direct_byte_bitwise_operand(left) {
            return false;
        }
        let right_is_direct = self.is_direct_byte_bitwise_operand(right);
        let right_is_preparable = self.is_pointer_preparable_byte_lvalue(right);
        if target.space == AddressSpace::AbsoluteX && right_is_preparable {
            return false;
        }
        if !right_is_direct && !right_is_preparable {
            return false;
        }

        self.emit_lda_slot_byte(target, 0);
        self.emit_direct_byte_bitwise_operand(op, left);
        if right_is_direct {
            self.emit_direct_byte_bitwise_operand(next_op, right);
        } else {
            self.emit_sta_zero_page(runtime_zp::VALUE_TEMP);
            let pointer = if target.space == AddressSpace::IndirectIndexedY
                && target.zero_page_byte(0) == runtime_zp::ARRAY_ADDR
            {
                runtime_zp::ELEMENT_ADDR
            } else {
                runtime_zp::ARRAY_ADDR
            };
            let Some(right_slot) = self.reusable_lvalue_slot_with_pointer(right, pointer) else {
                return false;
            };
            if right_slot.size != 1 {
                return false;
            }
            self.emit_lda_zero_page(runtime_zp::VALUE_TEMP);
            self.emit_bitwise_slot_byte(next_op, right_slot, 0);
        }
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_expanded_compound_bitwise_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        target_slot: StorageSlot,
    ) -> bool {
        if !self.segment_storage || target_slot.size != 1 {
            return false;
        }
        let ExprKind::Binary {
            op: next_op,
            left,
            right,
        } = &value.kind
        else {
            return false;
        };
        if BinaryBitwiseOp::from_binary(*next_op).is_none() {
            return false;
        }
        let ExprKind::Binary {
            op,
            left: repeated_target,
            right: first_operand,
        } = &left.kind
        else {
            return false;
        };
        let Some(op) = BinaryBitwiseOp::from_binary(*op) else {
            return false;
        };
        if !Self::same_lvalue_expr(target, repeated_target) {
            return false;
        }

        let compound_value = Expr {
            kind: ExprKind::Binary {
                op: *next_op,
                left: first_operand.clone(),
                right: right.clone(),
            },
            text: value.text.clone(),
            span: value.span,
        };
        self.emit_compatible_byte_compound_bitwise_chain(target_slot, op, &compound_value)
    }

    pub(super) fn is_byte_compound_bitwise_chain(&self, value: &Expr) -> bool {
        let ExprKind::Binary { op, left, right } = &value.kind else {
            return false;
        };
        matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
            && self.is_direct_byte_bitwise_operand(left)
            && (self.is_direct_byte_bitwise_operand(right)
                || self.is_pointer_preparable_byte_lvalue(right))
    }

    pub(super) fn is_direct_byte_bitwise_operand(&self, expr: &Expr) -> bool {
        self.constant_u16(expr)
            .is_some_and(|value| value <= u16::from(u8::MAX))
            || self
                .direct_scalar_slot(expr)
                .is_some_and(|slot| slot.size == 1)
    }

    pub(super) fn is_pointer_preparable_byte_lvalue(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Index { .. } => self.expr_size(expr) == Some(1),
            ExprKind::Call { callee, args } => self.array_call_slot_size(callee, args) == Some(1),
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return false;
                };
                let Some(base_slot) = self.lookup_slot(name) else {
                    return false;
                };
                let Some(record) = base_slot.record else {
                    return false;
                };
                base_slot.pointee_size.is_some()
                    && self
                        .record_layouts
                        .field(record, field)
                        .is_some_and(|field| field.size == 1)
            }
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let ExprKind::Name(name) = &expr.kind else {
                    return false;
                };
                self.lookup_slot(name)
                    .and_then(|slot| slot.pointee_size)
                    .is_some_and(|size| size == 1)
            }
            _ => false,
        }
    }

    pub(super) fn emit_direct_byte_bitwise_operand(&mut self, op: BinaryBitwiseOp, expr: &Expr) {
        if let Some(value) = self.constant_u16(expr) {
            let immediate = Immediate::new(value);
            self.emit_bitwise_immediate(op, immediate, 0);
            return;
        }
        let slot = self
            .direct_scalar_slot(expr)
            .expect("direct byte bitwise operand");
        debug_assert_eq!(slot.size, 1);
        self.emit_bitwise_slot_byte(op, slot, 0);
    }

    pub(super) fn inline_byte_array_direct_index(
        &self,
        expr: &Expr,
    ) -> Option<(StorageSlot, StorageSlot)> {
        let (base, index) = match &expr.kind {
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            _ => return None,
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let array = self.lookup_slot(name)?;
        if array.array != Some(ArrayStorage::Inline) || array.size != 1 {
            return None;
        }
        let index = self.direct_scalar_slot(index)?;
        (index.size == 1).then_some((array, index))
    }

    pub(super) fn compatible_compound_target_slot(&mut self, target: &Expr) -> Option<StorageSlot> {
        if self.profile.enables_modern_optimizations()
            && self.prepared_pointer_fact(target).is_some()
            && let Some(slot) =
                self.reusable_lvalue_slot_with_pointer_or_direct(target, runtime_zp::ARRAY_ADDR)
        {
            return Some(slot);
        }
        if let Some(slot) = self.lvalue_slot(target) {
            return Some(slot);
        }

        let ExprKind::Name(name) = &target.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if slot.array == Some(ArrayStorage::Pointer) {
            return Some(StorageSlot {
                size: 2,
                array: None,
                ..slot
            });
        }
        None
    }

    pub(super) fn emit_bitwise_operand_to_slot(&mut self, expr: &Expr, slot: StorageSlot) -> bool {
        let previous = self.inline_byte_constant_shift;
        self.inline_byte_constant_shift = true;
        let emitted = self.emit_expr_to_slot(expr, slot);
        self.inline_byte_constant_shift = previous;
        emitted
    }
}
