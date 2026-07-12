use super::*;

impl Generator {
    pub(super) fn emit_compatible_inline_byte_array_constant_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        let Some(value) = self.constant_u16(value) else {
            return false;
        };
        if !self.segment_storage || value > u16::from(u8::MAX) {
            return false;
        }
        let (base, index) = match &target.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return false,
        };
        let ExprKind::Name(name) = &base.kind else {
            return false;
        };
        let Some(array) = self.lookup_slot(name) else {
            return false;
        };
        if array.array != Some(ArrayStorage::Inline) || array.size != 1 {
            return false;
        }
        let Some(index) = self.constant_u16(index) else {
            return false;
        };
        let slot = StorageSlot {
            array: None,
            ..array.offset_bytes(index)
        };
        self.emit_lda_imm(value as u8);
        self.emit_sta_slot_byte(slot, 0);
        true
    }

    pub(super) fn emit_inline_byte_array_call_index_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        let Some((array, index)) = self.inline_byte_array_call_index(target) else {
            return false;
        };
        if !self.expr_preserves_call_return_slot(value) {
            return false;
        }
        let ExprKind::Call { callee, args } = &index.kind else {
            return false;
        };
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };
        if return_slot.size != 1 || !self.emit_call(callee, args, index.span) {
            return false;
        }
        if !self.emit_load_simple_byte(value, 0) {
            return false;
        }
        self.emit_ldx_slot_byte(return_slot, 0);
        self.emitter
            .emit_sta_absolute_x(AbsoluteX::new(array.address));
        true
    }

    pub(super) fn emit_inline_byte_array_same_index_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        let Some((target_array, target_index)) = self.inline_byte_array_scalar_index(target) else {
            return false;
        };
        let Some((source_array, source_index)) = self.inline_byte_array_scalar_index(value) else {
            return false;
        };
        if target_index != source_index {
            return false;
        }

        self.emit_ldx_slot_byte(target_index, 0);
        self.emitter
            .emit_lda_absolute_x(AbsoluteX::new(source_array.address));
        self.emitter
            .emit_sta_absolute_x(AbsoluteX::new(target_array.address));
        true
    }

    pub(super) fn emit_inline_byte_array_same_index_add_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        let Some((target_array, target_index)) = self.inline_byte_array_scalar_index(target) else {
            return false;
        };
        let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = &value.kind
        else {
            return false;
        };
        let Some((source_array, source_index)) = self.inline_byte_array_scalar_index(left) else {
            return false;
        };
        if target_index != source_index {
            return false;
        }
        let Some(right) = self.constant_index_byte_slot(right) else {
            return false;
        };

        self.emit_ldx_slot_byte(target_index, 0);
        self.emit_clc();
        self.emitter
            .emit_lda_absolute_x(AbsoluteX::new(source_array.address));
        self.emit_adc_slot_byte(right, 0);
        self.emitter
            .emit_sta_absolute_x(AbsoluteX::new(target_array.address));
        true
    }

    pub(super) fn constant_index_byte_slot(&mut self, expr: &Expr) -> Option<StorageSlot> {
        match &expr.kind {
            ExprKind::Index { base, index } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let array = self.lookup_slot(name)?;
                if array.size != 1 {
                    return None;
                }
                let index = self.constant_u16(index)?;
                self.constant_index_slot(array, index)
            }
            ExprKind::Call { callee, args } if args.len() == 1 => {
                let ExprKind::Name(name) = &callee.kind else {
                    return None;
                };
                let array = self.lookup_slot(name)?;
                if array.size != 1 {
                    return None;
                }
                let index = self.constant_u16(&args[0])?;
                self.constant_index_slot(array, index)
            }
            _ => self
                .reusable_lvalue_slot(expr)
                .filter(|slot| slot.size == 1),
        }
    }

    pub(super) fn emit_scalar_plus_inline_byte_array_index_assignment(
        &mut self,
        target: StorageSlot,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage
            || target.size != 1
            || target.array.is_some()
            || !matches!(
                target.space,
                AddressSpace::Absolute | AddressSpace::ZeroPage
            )
        {
            return false;
        }
        let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = &value.kind
        else {
            return false;
        };
        let Some(left) = self.direct_scalar_slot(left) else {
            return false;
        };
        if left.size != 1 || left.array.is_some() {
            return false;
        }
        let Some((array, index)) = self.inline_byte_array_scalar_index(right) else {
            return false;
        };

        self.emit_clc();
        self.emit_lda_slot_byte(left, 0);
        self.emit_ldx_slot_byte(index, 0);
        self.emitter
            .emit_adc_absolute_x(AbsoluteX::new(array.address));
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn emit_absolute_x_rhs_assignment_preserving_scalar_index(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage {
            return false;
        }
        let Some((array, index)) = self.inline_byte_array_scalar_index(target) else {
            return false;
        };
        if !self.expr_may_clobber_x_except_index(value, Some(index)) {
            return false;
        }

        let temp = StorageSlot::zero_page(runtime_zp::ARGS.address(), 1);
        if self.profile.enables_modern_optimizations()
            && self.call_expr_preserves_slot_byte(value, index, 0)
        {
            if !self.emit_expr_to_slot(value, temp) {
                return false;
            }
            self.emit_ldx_slot_byte(index, 0);
            self.emit_lda_slot_byte(temp, 0);
            self.emitter
                .emit_sta_absolute_x(AbsoluteX::new(array.address));
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                Some(value.span),
                format!(
                    "reloaded index from ${:04X} after preserving call instead of stack save",
                    index.byte_address(0)
                ),
            );
            return true;
        }
        self.emit_lda_slot_byte(index, 0);
        self.emitter.emit_pha();
        if !self.emit_expr_to_slot(value, temp) {
            return false;
        }
        self.emit_pla();
        self.emit_tax();
        self.emit_lda_slot_byte(temp, 0);
        self.emitter
            .emit_sta_absolute_x(AbsoluteX::new(array.address));
        true
    }

    pub(super) fn absolute_x_target_index_slot(&self, target: &Expr) -> Option<StorageSlot> {
        let (base, index) = match &target.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return None,
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let base_slot = self.lookup_slot(name)?;
        (self.segment_storage
            && base_slot.array == Some(ArrayStorage::Inline)
            && base_slot.size == 1)
            .then(|| self.direct_scalar_slot(index))?
    }

    pub(super) fn inline_byte_array_scalar_index(
        &self,
        expr: &Expr,
    ) -> Option<(StorageSlot, StorageSlot)> {
        let (base, index) = match &expr.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
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

    pub(super) fn inline_byte_array_call_index<'a>(
        &self,
        expr: &'a Expr,
    ) -> Option<(StorageSlot, &'a Expr)> {
        let (base, index) = match &expr.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return None,
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let array = self.lookup_slot(name)?;
        if array.array != Some(ArrayStorage::Inline) || array.size != 1 {
            return None;
        }
        let ExprKind::Call { callee, args } = &index.kind else {
            return None;
        };
        if self.array_call_slot_size(callee, args).is_some() {
            return None;
        }
        let return_slot = self.call_return_slot(callee)?;
        (return_slot.size == 1).then_some((array, index))
    }

    pub(super) fn expr_may_clobber_x_except_index(
        &self,
        expr: &Expr,
        preserved_index: Option<StorageSlot>,
    ) -> bool {
        match &expr.kind {
            ExprKind::Call { callee, args }
                if self.array_call_slot_size(callee, args).is_some() =>
            {
                args.len() != 1 || self.direct_scalar_slot(&args[0]) != preserved_index
            }
            ExprKind::Call { .. } => true,
            ExprKind::Index { index, .. } => self.direct_scalar_slot(index) != preserved_index,
            ExprKind::Field { .. } => false,
            ExprKind::Unary { expr, .. } => {
                self.expr_may_clobber_x_except_index(expr, preserved_index)
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_may_clobber_x_except_index(left, preserved_index)
                    || self.expr_may_clobber_x_except_index(right, preserved_index)
            }
            _ => false,
        }
    }

    pub(super) fn expr_may_prepare_indirect_slot(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => true,
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                ..
            } => false,
            ExprKind::Unary { expr, .. } => Self::expr_contains_indirect_lvalue(expr),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_contains_indirect_lvalue(left)
                    || Self::expr_contains_indirect_lvalue(right)
            }
            _ => false,
        }
    }

    pub(super) fn expr_contains_indirect_lvalue(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Index { .. } => true,
            ExprKind::Call { args, .. } => args.len() == 1,
            ExprKind::Field { .. } => true,
            ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => true,
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                ..
            } => false,
            ExprKind::Unary { expr, .. } => Self::expr_contains_indirect_lvalue(expr),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_contains_indirect_lvalue(left)
                    || Self::expr_contains_indirect_lvalue(right)
            }
            _ => false,
        }
    }

    pub(super) fn emit_routine_target_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        span: Span,
    ) -> bool {
        if !self.segment_storage {
            return false;
        }
        let ExprKind::Name(target_name) = &target.kind else {
            return false;
        };
        let Some(target_info) = self.routines.get(&normalize_name(target_name)) else {
            return false;
        };
        if target_info.system_address.is_some() {
            return false;
        }

        let ExprKind::Name(value_name) = &value.kind else {
            return false;
        };
        let Some(value_info) = self.routines.get(&normalize_name(value_name)).cloned() else {
            return false;
        };

        let target_low = routine_trampoline_operand_label(target_name, 0);
        let target_high = routine_trampoline_operand_label(target_name, 1);
        if let Some(address) = value_info.system_address {
            let immediate = Immediate::new(address);
            self.emit_lda_immediate(immediate, 1);
            self.emit_sta_absolute_label(target_high, span);
            self.emit_lda_immediate(immediate, 0);
            self.emit_sta_absolute_label(target_low, span);
        } else {
            self.emit_lda_label_high(value_info.label.clone(), span);
            self.emit_sta_absolute_label(target_high, span);
            self.emit_lda_label_low(value_info.label, span);
            self.emit_sta_absolute_label(target_low, span);
        }
        self.processor.invalidate_index_y();
        self.straight_line_store_y = None;
        true
    }

    pub(super) fn emit_routine_name_to_card_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        span: Span,
    ) -> bool {
        if !self.segment_storage {
            return false;
        }
        let ExprKind::Name(value_name) = &value.kind else {
            return false;
        };
        let Some(value_info) = self.routines.get(&normalize_name(value_name)).cloned() else {
            return false;
        };

        let Some(slot) = self.lvalue_slot(target) else {
            return false;
        };
        if slot.size != 2 || slot.pointee_size.is_some() || slot.array.is_some() {
            return false;
        }

        if let Some(address) = value_info.system_address {
            let immediate = Immediate::new(address);
            self.emit_lda_immediate(immediate, 1);
            self.emit_sta_slot_byte(slot, 1);
            self.emit_lda_immediate(immediate, 0);
            self.emit_sta_slot_byte(slot, 0);
        } else if self.profile.enables_modern_optimizations() {
            self.emit_lda_label_high(value_info.label.clone(), span);
            self.emit_sta_slot_byte(slot, 1);
            self.emit_lda_label_low(value_info.label, span);
            self.emit_sta_slot_byte(slot, 0);
        } else {
            self.emit_lda_absolute_label(routine_trampoline_operand_label(value_name, 1), span);
            self.emit_sta_slot_byte(slot, 1);
            self.emit_lda_absolute_label(routine_trampoline_operand_label(value_name, 0), span);
            self.emit_sta_slot_byte(slot, 0);
        }
        self.processor.invalidate_index_y();
        self.straight_line_store_y = None;
        true
    }

    pub(super) fn emit_compatible_inline_byte_array_dynamic_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage {
            return false;
        }
        let (base, index) = match &target.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return false,
        };
        let ExprKind::Name(name) = &base.kind else {
            return false;
        };
        let Some(array) = self.lookup_slot(name) else {
            return false;
        };
        if array.array != Some(ArrayStorage::Inline) || array.size != 1 {
            return false;
        }
        let Some(index_slot) = self.direct_scalar_slot(index) else {
            return false;
        };
        if index_slot.size != 1 {
            return false;
        }
        let Some(value_slot) = self.direct_scalar_slot(value) else {
            return false;
        };
        if value_slot.size != 1 {
            return false;
        }

        self.emit_lda_slot_byte(value_slot, 0);
        self.emit_ldx_slot_byte(index_slot, 0);
        self.emitter
            .emit_sta_absolute_x(AbsoluteX::new(array.address));
        true
    }

    pub(super) fn emit_indirect_call_assignment_preserving_pointer(
        &mut self,
        slot: StorageSlot,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage || slot.space != AddressSpace::IndirectIndexedY {
            return false;
        }
        let ExprKind::Call { callee, args } = &value.kind else {
            return false;
        };
        if self.array_call_slot_size(callee, args).is_some() {
            return false;
        }
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };

        let pointer = slot.zero_page_byte(0);
        let arguments_may_clobber_pointer = args.iter().any(Self::expr_contains_indirect_lvalue);
        let preserve_pointer = self
            .call_target_effects(callee)
            .is_none_or(|effects| !effects.known || effects.writes_pointer_pair(pointer))
            || arguments_may_clobber_pointer;
        if preserve_pointer
            && self.emit_indirect_call_assignment_with_alternate_arg_pointer(
                slot,
                callee,
                args,
                return_slot,
                value.span,
            )
        {
            return true;
        }
        if preserve_pointer {
            self.emit_lda_zero_page(pointer.offset(1));
            self.emitter.emit_pha();
            self.emit_lda_zero_page(pointer);
            self.emitter.emit_pha();
        } else {
            self.record_modern_optimization(
                CodegenOptimizationKind::PointerReloadRemoved,
                12,
                Some(value.span),
                format!(
                    "skipped preserving ${:02X}/${:02X} around call with known effects",
                    pointer.address(),
                    pointer.offset(1).address()
                ),
            );
        }
        if !self.emit_call(callee, args, value.span) {
            return false;
        }
        if preserve_pointer {
            self.emit_pla();
            self.emit_sta_zero_page(pointer);
            self.emit_pla();
            self.emit_sta_zero_page(pointer.offset(1));
        }
        self.emit_copy_slot_to_slot(return_slot, slot)
    }

    fn emit_indirect_call_assignment_with_alternate_arg_pointer(
        &mut self,
        target: StorageSlot,
        callee: &Expr,
        args: &[Expr],
        return_slot: StorageSlot,
        span: Span,
    ) -> bool {
        if !self.profile.enables_modern_optimizations()
            || target.size != 1
            || return_slot.size != 1
            || args.len() != 1
        {
            return false;
        }
        let ExprKind::Name(name) = &callee.kind else {
            return false;
        };
        let Some(info) = self.routines.get(&normalize_name(name)).cloned() else {
            return false;
        };
        if info.params.first().is_none_or(|slot| slot.size != 1)
            || !info.effects.known
            || info.effects.writes_pointer_pair(target.zero_page_byte(0))
        {
            return false;
        }
        let source_pointer = if target.zero_page_byte(0) == runtime_zp::ARRAY_ADDR {
            runtime_zp::ELEMENT_ADDR
        } else {
            runtime_zp::ARRAY_ADDR
        };
        let Some(source) = self.reusable_lvalue_slot_with_pointer(&args[0], source_pointer) else {
            return false;
        };
        if source.size != 1 {
            return false;
        }

        self.emit_lda_slot_byte(source, 0);
        self.emit_call_target(&info, span, false);
        self.emit_copy_slot_to_slot(return_slot, target);
        self.record_modern_optimization(
            CodegenOptimizationKind::PointerReloadRemoved,
            12,
            Some(span),
            format!(
                "prepared call argument through ${:02X}/${:02X} to preserve target ${:02X}/${:02X}",
                source_pointer.address(),
                source_pointer.offset(1).address(),
                target.zero_page_byte(0).address(),
                target.zero_page_byte(0).offset(1).address()
            ),
        );
        true
    }

    pub(super) fn emit_indirect_self_word_negation_assignment(
        &mut self,
        target_expr: &Expr,
        value: &Expr,
        target: StorageSlot,
    ) -> bool {
        if !self.segment_storage
            || target.space != AddressSpace::IndirectIndexedY
            || target.size != 2
        {
            return false;
        }
        let ExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } = &value.kind
        else {
            return false;
        };
        if !Self::same_lvalue_expr(target_expr, expr) {
            return false;
        }

        let source_pointer = if target.zero_page_byte(0) == runtime_zp::ARRAY_ADDR {
            runtime_zp::ELEMENT_ADDR
        } else {
            runtime_zp::ARRAY_ADDR
        };
        let Some(source) = self.reusable_lvalue_slot_with_pointer(expr, source_pointer) else {
            return false;
        };
        let source = source.signed(true);
        debug_assert_indirect_slots_do_not_alias(source, target, "indirect self negation");

        self.emit_sec();
        self.emit_lda_imm(0);
        self.emit_ldy_imm(0);
        self.emit_sbc_slot_byte(source, 0);
        self.emit_sta_zero_page(runtime_zp::VALUE_TEMP);
        self.emit_lda_imm(0);
        self.emit_iny();
        self.emit_sbc_slot_byte(source, 1);
        self.emit_sta_slot_byte(target, 1);
        self.emit_lda_zero_page(runtime_zp::VALUE_TEMP);
        self.emit_dey();
        self.emit_sta_slot_byte(target, 0);
        true
    }

    pub(super) fn same_lvalue_expr(left: &Expr, right: &Expr) -> bool {
        match (&left.kind, &right.kind) {
            (ExprKind::Name(left), ExprKind::Name(right)) => {
                normalize_name(left) == normalize_name(right)
            }
            (
                ExprKind::Unary {
                    op: left_op,
                    expr: left,
                },
                ExprKind::Unary {
                    op: right_op,
                    expr: right,
                },
            ) if left_op == right_op => Self::same_lvalue_expr(left, right),
            (
                ExprKind::Index {
                    base: left_base,
                    index: left_index,
                },
                ExprKind::Index {
                    base: right_base,
                    index: right_index,
                },
            ) => {
                Self::same_lvalue_expr(left_base, right_base)
                    && Self::same_lvalue_expr(left_index, right_index)
            }
            (
                ExprKind::Call {
                    callee: left_base,
                    args: left_args,
                },
                ExprKind::Call {
                    callee: right_base,
                    args: right_args,
                },
            ) if left_args.len() == 1 && right_args.len() == 1 => {
                Self::same_lvalue_expr(left_base, right_base)
                    && Self::same_lvalue_expr(&left_args[0], &right_args[0])
            }
            (
                ExprKind::Call {
                    callee: left_base,
                    args: left_args,
                },
                ExprKind::Index {
                    base: right_base,
                    index: right_index,
                },
            ) if left_args.len() == 1 => {
                Self::same_lvalue_expr(left_base, right_base)
                    && Self::same_lvalue_expr(&left_args[0], right_index)
            }
            (
                ExprKind::Index {
                    base: left_base,
                    index: left_index,
                },
                ExprKind::Call {
                    callee: right_base,
                    args: right_args,
                },
            ) if right_args.len() == 1 => {
                Self::same_lvalue_expr(left_base, right_base)
                    && Self::same_lvalue_expr(left_index, &right_args[0])
            }
            (
                ExprKind::Binary {
                    op: left_op,
                    left: left_left,
                    right: left_right,
                },
                ExprKind::Binary {
                    op: right_op,
                    left: right_left,
                    right: right_right,
                },
            ) if left_op == right_op => {
                Self::same_lvalue_expr(left_left, right_left)
                    && Self::same_lvalue_expr(left_right, right_right)
            }
            _ => false,
        }
    }

    pub(super) fn emit_indirect_rhs_assignment_preserving_pointer(
        &mut self,
        target_expr: &Expr,
        target: StorageSlot,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage || target.space != AddressSpace::IndirectIndexedY {
            return false;
        }
        let needs_preservation = if target.size == 1 {
            Self::expr_contains_indirect_lvalue(value)
        } else {
            Self::expr_may_prepare_indirect_slot(value)
        };
        if !needs_preservation {
            return false;
        }

        let pointer = target.zero_page_byte(0);
        let source_pointer = if pointer == runtime_zp::ARRAY_ADDR {
            runtime_zp::ELEMENT_ADDR
        } else {
            runtime_zp::ARRAY_ADDR
        };
        let alternate_source_pointer =
            if pointer != runtime_zp::VALUE_TEMP && source_pointer != runtime_zp::VALUE_TEMP {
                runtime_zp::VALUE_TEMP
            } else {
                runtime_zp::ADDR
            };
        if target.size == 1 {
            if let Some(source) = self.reusable_lvalue_slot_with_pointer(value, source_pointer) {
                self.emit_lda_slot_byte(source, 0);
                self.emit_sta_slot_byte(target, 0);
                return true;
            }
            if let ExprKind::Binary { op, left, right } = &value.kind {
                if self.emit_indirect_byte_lvalue_arithmetic_to_slot(
                    *op,
                    target_expr,
                    left,
                    right,
                    target,
                    source_pointer,
                ) {
                    return true;
                }
                if *op == BinaryOp::Add
                    && self.emit_indirect_byte_lvalue_arithmetic_to_slot(
                        *op,
                        target_expr,
                        right,
                        left,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if self.emit_indirect_byte_lvalue_lvalue_bitwise_to_slot(
                    *op,
                    left,
                    right,
                    target,
                    source_pointer,
                    alternate_source_pointer,
                ) {
                    return true;
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                    && self.emit_indirect_byte_lvalue_lvalue_bitwise_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                        alternate_source_pointer,
                    )
                {
                    return true;
                }
                if !Self::arithmetic_operand_needs_materialization(right)
                    && self.emit_indirect_byte_lvalue_simple_arithmetic_to_slot(
                        *op,
                        left,
                        right,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if *op == BinaryOp::Add
                    && !Self::arithmetic_operand_needs_materialization(left)
                    && self.emit_indirect_byte_lvalue_simple_arithmetic_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if !Self::arithmetic_operand_needs_materialization(right)
                    && self.emit_indirect_byte_lvalue_simple_bitwise_to_slot(
                        *op,
                        left,
                        right,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                    && !Self::arithmetic_operand_needs_materialization(left)
                    && self.emit_indirect_byte_lvalue_simple_bitwise_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if self.emit_indirect_byte_lvalue_constant_arithmetic_to_slot(
                    *op,
                    left,
                    right,
                    target,
                    source_pointer,
                ) {
                    return true;
                }
                if *op == BinaryOp::Add
                    && self.emit_indirect_byte_lvalue_constant_arithmetic_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
            }
        }
        if target.size == 2 {
            if let ExprKind::Binary { op, left, right } = &value.kind {
                if self.emit_indirect_word_lvalue_lvalue_arithmetic_to_slot(
                    *op,
                    left,
                    right,
                    target,
                    source_pointer,
                    alternate_source_pointer,
                ) {
                    return true;
                }
                if *op == BinaryOp::Add
                    && self.emit_indirect_word_lvalue_lvalue_arithmetic_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                        alternate_source_pointer,
                    )
                {
                    return true;
                }
                if self.emit_indirect_word_lvalue_lvalue_bitwise_to_slot(
                    *op,
                    left,
                    right,
                    target,
                    source_pointer,
                    alternate_source_pointer,
                ) {
                    return true;
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                    && self.emit_indirect_word_lvalue_lvalue_bitwise_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                        alternate_source_pointer,
                    )
                {
                    return true;
                }
                if self.emit_indirect_word_lvalue_byte_arithmetic_to_slot(
                    *op,
                    left,
                    right,
                    target,
                    source_pointer,
                    alternate_source_pointer,
                ) {
                    return true;
                }
                if *op == BinaryOp::Add
                    && self.emit_indirect_word_lvalue_byte_arithmetic_to_slot(
                        *op,
                        right,
                        left,
                        target,
                        source_pointer,
                        alternate_source_pointer,
                    )
                {
                    return true;
                }
                if *op == BinaryOp::Add
                    && !Self::arithmetic_operand_needs_materialization(right)
                    && self.emit_add_indexed_word_expr_to_slot_with_pointer(
                        left,
                        right,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if *op == BinaryOp::Add
                    && !Self::arithmetic_operand_needs_materialization(left)
                    && self.emit_add_indexed_word_expr_to_slot_with_pointer(
                        right,
                        left,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if *op == BinaryOp::Add
                    && let Some(value) = self.constant_u16(right)
                    && self.emit_add_constant_indexed_word_to_slot_with_pointer(
                        left,
                        value,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
                if *op == BinaryOp::Add
                    && let Some(value) = self.constant_u16(left)
                    && self.emit_add_constant_indexed_word_to_slot_with_pointer(
                        right,
                        value,
                        target,
                        source_pointer,
                    )
                {
                    return true;
                }
            }
            if let Some(source) = self.reusable_lvalue_slot_with_pointer(value, source_pointer) {
                self.emit_lda_slot_byte(source, 1);
                self.emit_sta_slot_byte(target, 1);
                self.emit_lda_slot_byte(source, 0);
                self.emit_sta_slot_byte(target, 0);
                return true;
            }
        }

        let temp = StorageSlot::zero_page(runtime_zp::ARGS.address(), target.size);
        self.emit_lda_zero_page(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page(pointer);
        self.emitter.emit_pha();
        if !self.emit_expr_to_slot(value, temp) {
            return false;
        }
        self.emit_pla();
        self.emit_sta_zero_page(pointer);
        self.emit_pla();
        self.emit_sta_zero_page(pointer.offset(1));
        self.emit_copy_slot_to_slot(temp, target)
    }

    pub(super) fn emit_absolute_x_rhs_assignment_preserving_index(
        &mut self,
        target: StorageSlot,
        target_index: Option<StorageSlot>,
        value: &Expr,
    ) -> bool {
        if !self.segment_storage || target.space != AddressSpace::AbsoluteX {
            return false;
        }
        if !self.expr_may_clobber_x_except_index(value, target_index) {
            return false;
        }

        let temp = StorageSlot::zero_page(runtime_zp::ARGS.address(), target.size);
        self.emit_txa();
        self.emitter.emit_pha();
        if !self.emit_expr_to_slot(value, temp) {
            return false;
        }
        self.emit_pla();
        self.emit_tax();
        self.emit_copy_slot_to_slot(temp, target)
    }

    fn emit_store_prepared_indirect_y(&mut self, pointer: ZeroPage) {
        self.emitter
            .emit_sta_indirect_indexed_y(IndirectIndexedY::new(pointer));
        self.processor.invalidate_memory();
    }

    pub(super) fn emit_effective_address_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
    ) -> bool {
        let Some(address) = self.byte_index_effective_address(target, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        let source = if let Some(value) = self.constant_u16(value) {
            EffectiveAddressStoreSource::Constant(value)
        } else {
            let Some(slot) = self.direct_scalar_slot(value) else {
                return false;
            };
            if slot_overlaps_zero_page(slot, address.pointer, 2) {
                return false;
            }
            EffectiveAddressStoreSource::Slot(slot)
        };

        if !self.emit_effective_address_pointer_and_y(address, 0) {
            return false;
        }
        self.emit_effective_address_store_source_byte(source, 0);
        self.emit_store_prepared_indirect_y(address.pointer);
        if address.element_size > 1 {
            self.emit_iny();
            self.emit_effective_address_store_source_byte(source, 1);
            self.emit_store_prepared_indirect_y(address.pointer);
        }
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressLowered,
            if address.element_size == 1 { 4 } else { 8 },
            Some(target.span),
            "stored through byte-indexed effective address without materializing element pointer",
        );
        true
    }

    pub(super) fn emit_same_effective_address_call_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        span: Span,
    ) -> bool {
        let ExprKind::Call { callee, args } = &value.kind else {
            return false;
        };
        if args.len() != 1 || !self.same_effective_address(target, &args[0]) {
            return false;
        }
        let Some(address) = self.byte_index_effective_address(target, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if address.element_size != 1 {
            return false;
        }
        let Some(info) = self.call_routine_info(callee) else {
            return false;
        };
        if info.params.len() != 1
            || info.params[0].size != 1
            || info.return_slot.is_none_or(|slot| slot.size != 1)
            || !info.effects.known
            || info.effects.writes_pointer_pair(address.pointer)
            || !effects_preserve_slot_byte(info.effects, address.index, 0)
        {
            return false;
        }

        if !self.emit_effective_address_pointer_and_y(address, 0) {
            return false;
        }
        self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(address.pointer));
        self.emit_call_target(&info, value.span, false);
        if !info.facts.returns_a_equals_a0 {
            self.emit_lda_slot_byte(info.return_slot.unwrap(), 0);
        }
        self.emit_ldy_slot_byte(address.index, 0);
        self.emit_store_prepared_indirect_y(address.pointer);
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressReused,
            10,
            Some(span),
            "reused same byte-indexed effective address across preserving call assignment",
        );
        true
    }

    fn same_effective_address(&self, left: &Expr, right: &Expr) -> bool {
        let Some(left) = self.byte_index_effective_address(left, runtime_zp::ARRAY_ADDR) else {
            return false;
        };
        let Some(right) = self.byte_index_effective_address(right, runtime_zp::ARRAY_ADDR) else {
            return false;
        };
        left.base == right.base
            && left.index == right.index
            && left.element_size == right.element_size
    }

    fn emit_effective_address_store_source_byte(
        &mut self,
        source: EffectiveAddressStoreSource,
        byte_index: u16,
    ) {
        match source {
            EffectiveAddressStoreSource::Constant(value) => {
                self.emit_lda_immediate(Immediate::new(value), byte_index);
            }
            EffectiveAddressStoreSource::Slot(slot) if byte_index < slot.size => {
                self.emit_lda_slot_byte_value_only(slot, byte_index);
            }
            EffectiveAddressStoreSource::Slot(_) => self.emit_lda_imm(0),
        }
    }

    pub(super) fn emit_array_name_assignment(&mut self, target: &Expr, value: &Expr) -> bool {
        let ExprKind::Name(name) = &target.kind else {
            return false;
        };
        let Some(slot) = self.lookup_slot(name) else {
            return false;
        };
        if slot.array != Some(ArrayStorage::Pointer) {
            return false;
        }

        match &value.kind {
            ExprKind::String(text) if slot.size == 1 => {
                let literal = self.emit_string_literal_storage(text, value.span);
                self.emit_store_array_pointer_address(slot, literal);
                true
            }
            _ => self.emit_copy_array_pointer_expr_to_slot(value, slot),
        }
    }

    fn emit_copy_array_pointer_expr_to_slot(&mut self, value: &Expr, target: StorageSlot) -> bool {
        if self.emit_load_array_pointer_value_byte(value, 0) {
            self.emit_sta_slot_byte(target, 0);
            if !self.emit_load_array_pointer_value_byte(value, 1) {
                return false;
            }
            self.emit_sta_slot_byte(target, 1);
            return true;
        }

        self.emit_expr_to_slot(
            value,
            StorageSlot {
                size: 2,
                array: None,
                ..target
            },
        )
    }
}
