use super::*;

pub(super) fn debug_assert_call_abi_shape(
    callee: &str,
    info: &RoutineInfo,
    supplied_arg_count: usize,
) {
    debug_assert!(
        supplied_arg_count <= info.params.len(),
        "call to `{callee}` supplies more arguments than ABI slots"
    );
    let mut expected_offset = 0u16;
    for slot in &info.params {
        debug_assert!(
            slot.size > 0,
            "call parameter ABI slot for `{callee}` must have a non-zero width"
        );
        debug_assert_eq!(
            slot.space,
            AddressSpace::ZeroPage,
            "call parameter ABI slot for `{callee}` must live in zero page"
        );
        debug_assert_eq!(
            slot.address,
            runtime_zp::ARGS.address() as u16 + expected_offset,
            "call parameter ABI slots for `{callee}` must be packed from ARGS"
        );
        debug_assert!(
            slot.array.is_none(),
            "call parameter ABI slot for `{callee}` must describe argument storage, not array backing"
        );
        expected_offset = expected_offset.saturating_add(slot.size);
        debug_assert!(
            runtime_zp::ARGS.address() as u16 + expected_offset <= 0x100,
            "call parameter ABI slots for `{callee}` must not wrap zero page"
        );
    }
}

pub(super) fn debug_assert_call_arg_byte_shape(
    slot: StorageSlot,
    target_offset: u8,
    byte_index: u16,
) {
    debug_assert!(
        byte_index < slot.size,
        "call argument byte index must stay inside the parameter slot"
    );
    debug_assert_eq!(
        slot.space,
        AddressSpace::ZeroPage,
        "call argument parameter slot must live in the ABI zero page"
    );
    debug_assert_eq!(
        u16::from(target_offset),
        slot.address
            .wrapping_sub(runtime_zp::ARGS.address() as u16)
            .wrapping_add(byte_index),
        "call argument target offset must match the parameter byte"
    );
    debug_assert!(
        target_offset < 3,
        "call register argument helper may only target A/X/Y ABI bytes"
    );
}

pub(super) fn debug_assert_call_arg_value_shape(
    arg: &Expr,
    slot: StorageSlot,
    byte_index: u16,
    literal_address: Option<Absolute>,
) {
    debug_assert!(
        byte_index < slot.size,
        "call argument load must stay inside the parameter slot"
    );
    debug_assert!(
        slot.size > 0,
        "call argument parameter slot must have a non-zero width"
    );
    if literal_address.is_some() {
        debug_assert!(
            slot.size >= 2,
            "string literal call argument requires a pointer-sized parameter"
        );
    }
    if matches!(arg.kind, ExprKind::String(_)) {
        debug_assert!(
            slot.size >= 2,
            "string call argument requires a pointer-sized parameter"
        );
    }
}

pub(super) fn debug_assert_call_return_slot_shape(callee: &str, slot: StorageSlot) {
    debug_assert!(
        matches!(slot.size, 1 | 2),
        "function `{callee}` return slot must be byte- or word-sized"
    );
    debug_assert_eq!(
        slot.space,
        AddressSpace::ZeroPage,
        "function `{callee}` return slot must live in the ABI zero page"
    );
    debug_assert_eq!(
        slot.address,
        runtime_zp::ARGS.address() as u16,
        "function `{callee}` return slot must start at ARGS"
    );
    debug_assert!(
        slot.pointee_size.is_none(),
        "function `{callee}` return slot must be scalar storage, not pointer storage"
    );
    debug_assert!(
        slot.array.is_none(),
        "function `{callee}` return slot must be scalar storage, not array storage"
    );
}

pub(super) fn expr_needs_call_staging(expr: &Expr) -> bool {
    if constant_u16(expr).is_some() {
        return false;
    }

    match &expr.kind {
        ExprKind::Call { .. } | ExprKind::Binary { .. } => true,
        ExprKind::Unary {
            op: UnaryOp::Neg, ..
        } => true,
        ExprKind::Unary { expr, .. } => expr_needs_call_staging(expr),
        ExprKind::Index { base, index } => {
            expr_needs_call_staging(base) || expr_needs_call_staging(index)
        }
        ExprKind::Field { base, .. } => expr_needs_call_staging(base),
        _ => false,
    }
}

pub(super) fn first_arg_is_pointer_deref(expr: &Expr) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Unary {
            op: UnaryOp::Deref,
            expr,
        } if matches!(&expr.kind, ExprKind::Name(_))
    )
}

impl Generator {
    pub(super) fn emit_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> bool {
        self.emit_call_or_tail_jump(callee, args, span, false)
    }

    pub(super) fn emit_tail_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> bool {
        self.emit_call_or_tail_jump(callee, args, span, true)
    }

    pub(super) fn can_emit_call_target(&self, callee: &Expr, args: &[Expr]) -> bool {
        let ExprKind::Name(name) = &callee.kind else {
            return false;
        };
        if self.callable_pointer_info(name).is_some() {
            return args.is_empty() && self.lookup_slot(name).is_some_and(|slot| slot.size == 2);
        }
        self.routines
            .get(&normalize_name(name))
            .is_some_and(|info| args.len() <= info.params.len())
    }

    pub(super) fn emit_call_target(&mut self, info: &RoutineInfo, span: Span, tail_jump: bool) {
        if tail_jump {
            if let Some(address) = info.system_address {
                self.emitter.emit_jmp_absolute(Absolute::new(address));
            } else {
                self.emitter.emit_jmp_label(info.label.clone(), span);
            }
        } else if let Some(address) = info.system_address {
            self.emitter.emit_jsr_absolute(Absolute::new(address));
        } else {
            self.emitter.emit_jsr_label(info.label.clone(), span);
        }
        self.merge_current_callee_effects(info.effects);
        if self.profile.enables_modern_optimizations() && info.effects.known {
            let preserved_registers = self
                .processor
                .register_facts_preserved_by_known_call(info.effects);
            if preserved_registers > 0 {
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallFactPreserved,
                    0,
                    Some(span),
                    format!(
                        "preserved {preserved_registers} register fact{} across annotated call",
                        if preserved_registers == 1 { "" } else { "s" }
                    ),
                );
            }
            let preserved_zp = self
                .processor
                .stable_zero_page_facts_preserved_by_known_call(info.effects);
            if preserved_zp > 0 {
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallFactPreserved,
                    0,
                    Some(span),
                    format!(
                        "preserved {preserved_zp} stable zero-page fact{} across known-effect call",
                        if preserved_zp == 1 { "" } else { "s" }
                    ),
                );
            }
            let preserved_memory = self
                .processor
                .stable_memory_facts_preserved_by_known_call(info.effects);
            if preserved_memory > 0 {
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallFactPreserved,
                    0,
                    Some(span),
                    format!(
                        "preserved {preserved_memory} stable memory fact{} across known-effect call",
                        if preserved_memory == 1 { "" } else { "s" }
                    ),
                );
            }
            self.processor.invalidate_after_known_call(info.effects);
        } else {
            self.processor.invalidate_after_call();
        }
        if !tail_jump {
            self.apply_call_return_facts(info);
        }
    }

    pub(super) fn apply_call_return_facts(&mut self, info: &RoutineInfo) {
        if info.facts.returns_a_equals_a0 {
            let return_low = StorageSlot::zero_page(runtime_zp::ARGS.address(), 1);
            let value = ValueFact::SlotByte {
                slot: return_low,
                byte_index: 0,
            };
            self.processor.set_memory_byte(return_low, 0, value);
            self.processor.set_a_fact(value);
            self.processor.set_zp_from_a(runtime_zp::ARGS);
        } else if info.facts.returns_a_equals_a1 {
            let return_high = StorageSlot::zero_page(runtime_zp::ARGS.offset(1).address(), 1);
            let value = ValueFact::SlotByte {
                slot: return_high,
                byte_index: 0,
            };
            self.processor.set_memory_byte(return_high, 0, value);
            self.processor.set_a_fact(value);
            self.processor.set_zp_from_a(runtime_zp::ARGS.offset(1));
        }
    }
}

impl Generator {
    pub(super) fn emit_call_or_tail_jump(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        span: Span,
        tail_jump: bool,
    ) -> bool {
        let ExprKind::Name(name) = &callee.kind else {
            return false;
        };
        let Some(info) = self.routines.get(&normalize_name(name)).cloned() else {
            return self.emit_indirect_call_or_tail_jump(name, args, span, tail_jump);
        };
        if args.len() > info.params.len() {
            return false;
        }
        debug_assert_call_abi_shape(name, &info, args.len());
        let supplied_params = &info.params[..args.len()];

        if args.iter().any(expr_needs_call_staging) {
            let staged_args = StagedCallArgs::new(args, supplied_params);

            if self.profile.enables_modern_optimizations()
                && args
                    .iter()
                    .any(|arg| expr_contains_routine_call(arg, &self.routines))
            {
                if !self.emit_modern_left_to_right_staged_call_arguments(args, supplied_params) {
                    return false;
                }
            } else {
                let mut register_plan = StagedCallRegisterPlan::default();
                // Routine calls are handled above with protective stack staging. Array-call syntax is
                // only indexing, so ordinary staged arguments still evaluate left-to-right.
                for spec in staged_args.iter() {
                    let arg = spec.expr;
                    let slot = spec.slot;
                    let arg_offset = spec.offset;
                    if register_plan.is_staged(arg_offset) {
                        continue;
                    }
                    let defer_modern_first_byte_arg =
                        self.can_defer_modern_first_byte_staged_register_arg(arg, slot, arg_offset);
                    let deferred_literal_address =
                        self.staged_string_literal_register_arg(arg, slot, arg_offset);
                    if arg_offset == 1
                        && slot.size == 2
                        && self.can_defer_staged_register_arg_byte(
                            arg,
                            slot,
                            0,
                            deferred_literal_address,
                        )
                        && self.can_defer_staged_register_arg_byte(
                            arg,
                            slot,
                            1,
                            deferred_literal_address,
                        )
                    {
                        register_plan.defer(arg_offset, arg, slot, deferred_literal_address);
                        continue;
                    }
                    if arg_offset == 0
                        && slot.size == 2
                        && self.can_defer_staged_register_arg_byte(
                            arg,
                            slot,
                            0,
                            deferred_literal_address,
                        )
                        && self.can_defer_staged_register_arg_byte(
                            arg,
                            slot,
                            1,
                            deferred_literal_address,
                        )
                    {
                        register_plan.defer(arg_offset, arg, slot, deferred_literal_address);
                        continue;
                    }
                    if slot.size == 1
                        && arg_offset < 3
                        && (self.can_defer_staged_register_arg_byte(arg, slot, 0, None)
                            || defer_modern_first_byte_arg)
                    {
                        if defer_modern_first_byte_arg {
                            self.record_modern_optimization(
                                CodegenOptimizationKind::ArgumentStoreRemoved,
                                2,
                                Some(arg.span),
                                "deferred first byte argument in A instead of staging through $A0",
                            );
                        }
                        register_plan.defer(arg_offset, arg, slot, None);
                        continue;
                    }
                    if self.segment_storage
                        && slot.size == 2
                        && arg_offset == 2
                        && self.can_defer_staged_register_arg_byte(arg, slot, 0, None)
                    {
                        register_plan.defer(arg_offset, arg, slot, None);
                        if !self.emit_load_call_arg_byte(arg, slot, 1, None) {
                            return false;
                        }
                        self.emit_sta_zero_page(runtime_zp::ARGS.offset(3));
                        continue;
                    }
                    let can_forward_first_word_with_y_arg =
                        self.staged_first_word_can_forward_with_y_arg(spec, &staged_args);
                    let can_forward_first_word_high_with_computed_y_arg = self
                        .staged_first_word_high_can_forward_with_computed_y_arg(spec, &staged_args);
                    let can_forward_first_word_high_with_late_args =
                        self.staged_first_word_high_can_forward_with_late_args(spec, &staged_args);
                    let can_forward_second_byte_with_y_arg =
                        self.staged_second_byte_can_forward_with_y_arg(spec, &staged_args);
                    if let ExprKind::String(text) = &arg.kind {
                        if !self.emit_string_literal_address_to_slot(text, arg.span, slot) {
                            return false;
                        }
                    } else if (self.segment_storage
                        && self.emit_byte_constant_shift_expr_to_slot(arg, slot))
                        || self.emit_record_value_address_to_slot(arg, slot)
                    {
                        continue;
                    } else if (staged_args.total_bytes != 3
                        || arg_offset != 0
                        || can_forward_first_word_with_y_arg)
                        && self.emit_staged_word_register_arg_transfer(
                            arg,
                            slot,
                            arg_offset,
                            staged_args.total_bytes,
                        )
                    {
                        register_plan.mark_preloaded_word(arg_offset);
                        continue;
                    } else if self.profile.enables_modern_optimizations()
                        && can_forward_first_word_high_with_late_args
                        && self.emit_staged_lvalue_word_to_x_a_with_late_constants(
                            arg,
                            spec,
                            &staged_args,
                            &mut register_plan,
                        )
                    {
                        continue;
                    } else if (can_forward_first_word_high_with_computed_y_arg
                        || can_forward_first_word_high_with_late_args)
                        && self.emit_staged_lvalue_word_high_to_x_low_to_args(
                            arg,
                            arg_offset,
                            can_forward_first_word_high_with_computed_y_arg,
                        )
                    {
                        register_plan.mark_preloaded(arg_offset.wrapping_add(1));
                        if can_forward_first_word_high_with_computed_y_arg {
                            register_plan.mark_stacked(arg_offset);
                        }
                        continue;
                    } else if (staged_args.total_bytes != 3
                        || arg_offset != 1
                        || can_forward_second_byte_with_y_arg)
                        && self.emit_final_staged_register_arg_transfer(
                            arg,
                            slot,
                            arg_offset,
                            staged_args.total_bytes,
                        )
                    {
                        register_plan.mark_preloaded(arg_offset);
                        continue;
                    } else if !self.emit_expr_to_slot(arg, slot) {
                        return false;
                    }
                }
                self.emit_load_staged_call_registers(staged_args.total_bytes, &register_plan);
            }
        } else {
            let literal_addresses = self.emit_string_literal_argument_storage(args);
            if !self.emit_call_arguments_to_abi(args, supplied_params, &literal_addresses) {
                return false;
            }
        }

        self.emit_call_target(&info, span, tail_jump);
        self.straight_line_store_y = None;
        true
    }

    fn emit_indirect_call_or_tail_jump(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        tail_jump: bool,
    ) -> bool {
        let Some(_pointer_info) = self.callable_pointer_info(name) else {
            return false;
        };
        if !args.is_empty() {
            return false;
        }
        let Some(slot) = self.lookup_slot(name).filter(|slot| slot.size == 2) else {
            return false;
        };

        if tail_jump {
            self.emitter.emit_jmp_indirect(slot.address);
            self.processor.invalidate_after_jump();
            self.straight_line_store_y = None;
            return true;
        }

        let trampoline_label = self.next_label("indirect-call");
        let after_label = self.next_label("after-indirect-call");
        self.emitter.emit_jsr_label(trampoline_label.clone(), span);
        self.emit_jmp_label(after_label.clone(), span);
        self.bind_codegen_label(trampoline_label, span);
        self.emitter.emit_jmp_indirect(slot.address);
        self.processor.invalidate_after_jump();
        self.bind_codegen_label(after_label, span);
        self.processor.invalidate_after_call();
        self.straight_line_store_y = None;
        true
    }

    pub(super) fn emit_modern_left_to_right_staged_call_arguments(
        &mut self,
        args: &[Expr],
        params: &[StorageSlot],
    ) -> bool {
        if args.len() == 1
            && params.len() == 1
            && self.emit_single_call_result_argument_to_abi(&args[0], params[0])
        {
            return true;
        }

        for (arg, slot) in args.iter().zip(params.iter().copied()) {
            if let ExprKind::String(text) = &arg.kind {
                if !self.emit_string_literal_address_to_slot(text, arg.span, slot) {
                    return false;
                }
            } else if self.emit_record_value_address_to_slot(arg, slot) {
            } else if !self.emit_expr_to_slot(arg, slot) {
                return false;
            }

            for byte_index in 0..slot.size {
                self.emit_lda_slot_byte(slot, byte_index);
                self.emitter.emit_pha();
            }
        }

        for slot in params.iter().rev().copied() {
            for byte_index in (0..slot.size).rev() {
                self.emit_pla();
                self.emit_sta_slot_byte(slot, byte_index);
            }
        }

        self.processor.reset();
        self.emit_load_staged_call_registers(
            params.iter().map(|slot| slot.size).sum(),
            &StagedCallRegisterPlan::default(),
        );
        true
    }

    pub(super) fn emit_single_call_result_argument_to_abi(
        &mut self,
        arg: &Expr,
        target: StorageSlot,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        let ExprKind::Call { callee, args } = &arg.kind else {
            return false;
        };
        if self.array_call_slot_size(callee, args).is_some() {
            return false;
        }
        let Some(info) = self.call_routine_info(callee) else {
            return false;
        };
        let Some(return_slot) = info.return_slot else {
            return false;
        };
        if !self.emit_call(callee, args, arg.span) {
            return false;
        }

        let zero_extend_first_register_arg = target.size == 2
            && return_slot.size == 1
            && target.space == AddressSpace::ZeroPage
            && target.zero_page_byte(0) == runtime_zp::ARGS;
        let copied_bytes = return_slot.size.min(target.size);
        if copied_bytes == return_slot.size && return_slot.size == target.size {
            if !self.emit_copy_call_return_slot_to_slot(return_slot, target, info.internal_abi()) {
                return false;
            }
        } else {
            for byte_index in 0..copied_bytes {
                if return_slot.byte_address(byte_index) != target.byte_address(byte_index)
                    || return_slot.space != target.space
                {
                    self.emit_copy_slot_byte_to_slot_byte(
                        return_slot,
                        byte_index,
                        target,
                        byte_index,
                    );
                }
            }
            if !zero_extend_first_register_arg {
                for byte_index in copied_bytes..target.size {
                    self.emit_lda_imm(0);
                    self.emit_sta_slot_byte(target, byte_index);
                }
            }
        }

        if zero_extend_first_register_arg {
            self.emit_ldx_imm(0);
            self.emit_lda_zero_page(runtime_zp::ARGS);
        } else {
            let plan = StagedCallRegisterPlan::default();
            self.emit_load_staged_call_registers(target.size, &plan);
        }
        self.record_modern_optimization(
            CodegenOptimizationKind::ArgumentStackForwarded,
            4,
            Some(arg.span),
            "forwarded single call result argument without stack staging",
        );
        true
    }

    pub(super) fn can_defer_staged_register_arg_byte(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        byte_index: u16,
        literal_address: Option<Absolute>,
    ) -> bool {
        self.call_arg_immediate_byte(arg, slot, byte_index, literal_address)
            .is_some()
            || self.call_arg_zero_page_byte(arg, byte_index).is_some()
            || self.call_arg_absolute_byte(arg, byte_index).is_some()
    }

    pub(super) fn staged_first_word_can_forward_with_y_arg(
        &self,
        spec: StagedCallArg<'_>,
        args: &StagedCallArgs<'_>,
    ) -> bool {
        spec.offset == 0
            && spec.slot.size == 2
            && args.total_bytes == 3
            && self.staged_y_arg_can_load_without_accumulator(args)
    }

    pub(super) fn staged_first_word_high_can_forward_with_computed_y_arg(
        &self,
        spec: StagedCallArg<'_>,
        args: &StagedCallArgs<'_>,
    ) -> bool {
        spec.offset == 0
            && spec.slot.size == 2
            && args.total_bytes == 3
            && self.staged_y_arg_can_compute_without_x(args)
    }

    pub(super) fn staged_first_word_high_can_forward_with_late_args(
        &self,
        spec: StagedCallArg<'_>,
        args: &StagedCallArgs<'_>,
    ) -> bool {
        spec.offset == 0
            && spec.slot.size == 2
            && args.total_bytes > 3
            && self.staged_y_arg_can_load_without_accumulator(args)
            && self.staged_late_args_can_store_without_x(args)
    }

    pub(super) fn staged_second_byte_can_forward_with_y_arg(
        &self,
        spec: StagedCallArg<'_>,
        args: &StagedCallArgs<'_>,
    ) -> bool {
        spec.offset == 1
            && spec.slot.size == 1
            && args.total_bytes == 3
            && self.staged_y_arg_can_load_without_accumulator(args)
    }

    pub(super) fn staged_y_arg_can_load_without_accumulator(
        &self,
        args: &StagedCallArgs<'_>,
    ) -> bool {
        args.specs
            .iter()
            .find(|spec| spec.offset == 2 && spec.slot.size == 1)
            .is_some_and(|spec| self.call_arg_y_can_load_without_accumulator(spec.expr, spec.slot))
    }

    pub(super) fn staged_y_arg_can_compute_without_x(&self, args: &StagedCallArgs<'_>) -> bool {
        args.specs
            .iter()
            .find(|spec| spec.offset == 2 && spec.slot.size == 1)
            .is_some_and(|spec| self.can_emit_modern_byte_expr_to_acc(spec.expr, true))
    }

    pub(super) fn staged_late_args_can_store_without_x(&self, args: &StagedCallArgs<'_>) -> bool {
        args.specs
            .iter()
            .filter(|spec| spec.offset >= 3)
            .all(|spec| spec.slot.size == 1 && self.constant_u16(spec.expr).is_some())
    }

    pub(super) fn call_arg_y_can_load_without_accumulator(
        &self,
        arg: &Expr,
        slot: StorageSlot,
    ) -> bool {
        self.constant_u16(arg).is_some()
            || self.array_argument_base(arg).is_some()
            || self.record_value_argument_base(arg, slot).is_some()
            || self.call_arg_zero_page_byte(arg, 0).is_some()
            || self.call_arg_absolute_byte(arg, 0).is_some()
    }

    pub(super) fn staged_string_literal_register_arg(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
    ) -> Option<Absolute> {
        if !self.segment_storage
            || slot.size != 2
            || !(arg_offset == 1
                || (arg_offset == 0 && self.profile.enables_modern_optimizations()))
        {
            return None;
        }
        let ExprKind::String(text) = &arg.kind else {
            return None;
        };
        Some(self.emit_string_literal_storage(text, arg.span))
    }

    pub(super) fn can_defer_modern_first_byte_staged_register_arg(
        &self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
    ) -> bool {
        if !self.profile.enables_modern_optimizations()
            || slot.size != 1
            || slot.space != AddressSpace::ZeroPage
            || slot.address != runtime_zp::ARGS.address() as u16
            || arg_offset != 0
            || expr_contains_routine_call(arg, &self.routines)
        {
            return false;
        }

        match &arg.kind {
            ExprKind::Binary { .. }
                if self.can_emit_modern_first_byte_arg_expr_to_acc(arg, slot) =>
            {
                true
            }
            ExprKind::Call { callee, args } => self.array_call_slot_size(callee, args).is_some(),
            ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => true,
            _ => false,
        }
    }

    pub(super) fn can_emit_modern_first_byte_arg_expr_to_acc(
        &self,
        arg: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if !self.profile.enables_modern_optimizations()
            || slot.size != 1
            || slot.space != AddressSpace::ZeroPage
            || slot.address != runtime_zp::ARGS.address() as u16
            || self.expr_size(arg) != Some(1)
        {
            return false;
        }

        self.can_emit_modern_byte_expr_to_acc(arg, true)
    }

    pub(super) fn can_emit_modern_byte_expr_to_acc(
        &self,
        arg: &Expr,
        require_byte_shape: bool,
    ) -> bool {
        if !self.profile.enables_modern_optimizations()
            || expr_contains_routine_call(arg, &self.routines)
            || (require_byte_shape && self.expr_size(arg) != Some(1))
        {
            return false;
        }

        match &arg.kind {
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                left,
                right,
            } => {
                self.can_emit_modern_simple_byte_operand(left)
                    && self.can_emit_modern_simple_byte_operand(right)
            }
            ExprKind::Binary {
                op: BinaryOp::Lsh | BinaryOp::Rsh,
                left,
                right,
            } => {
                self.can_emit_modern_simple_byte_operand(left)
                    && self.expr_size(left) == Some(1)
                    && self.constant_u16(right).is_some()
            }
            _ => false,
        }
    }

    fn can_emit_modern_simple_byte_operand(&self, expr: &Expr) -> bool {
        if self.constant_u16(expr).is_some() {
            return true;
        }
        if let ExprKind::Cast { expr, .. } = &expr.kind {
            return self.can_emit_modern_simple_byte_operand(expr);
        }
        self.direct_scalar_slot(expr)
            .is_some_and(|slot| slot.size >= 1)
    }

    pub(super) fn can_forward_final_staged_register_arg_to_transfer(
        &self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
        arg_bytes: u16,
    ) -> bool {
        self.profile.enables_modern_optimizations()
            && slot.size == 1
            && matches!(arg_offset, 1 | 2)
            && (u16::from(arg_offset) + 1 == arg_bytes || (arg_offset == 1 && arg_bytes == 3))
            && arg_bytes <= 3
            && !expr_contains_routine_call(arg, &self.routines)
    }

    pub(super) fn emit_final_staged_register_arg_transfer(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
        arg_bytes: u16,
    ) -> bool {
        if !self.can_forward_final_staged_register_arg_to_transfer(arg, slot, arg_offset, arg_bytes)
        {
            return false;
        }
        let loaded = self.emit_load_call_arg_byte(arg, slot, 0, None)
            || (self.can_emit_modern_byte_expr_to_acc(arg, false)
                && self.emit_modern_byte_expr_to_acc(arg));
        if !loaded {
            return false;
        }

        match arg_offset {
            1 => {
                self.emit_tax();
                self.record_modern_optimization(
                    CodegenOptimizationKind::ArgumentStoreRemoved,
                    3,
                    Some(arg.span),
                    "forwarded final second argument byte from A to X instead of staging through $A1",
                );
            }
            2 => {
                self.emit_tay();
                self.straight_line_store_y = None;
                self.record_modern_optimization(
                    CodegenOptimizationKind::ArgumentStoreRemoved,
                    3,
                    Some(arg.span),
                    "forwarded final third argument byte from A to Y instead of staging through $A2",
                );
            }
            _ => return false,
        }
        true
    }

    pub(super) fn can_forward_staged_word_arg_to_registers(
        &self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
        arg_bytes: u16,
    ) -> bool {
        self.profile.enables_modern_optimizations()
            && slot.size == 2
            && !expr_contains_routine_call(arg, &self.routines)
            && ((arg_offset == 0 && (arg_bytes == 2 || arg_bytes == 3))
                || (arg_offset == 1 && arg_bytes == 3))
    }

    pub(super) fn emit_staged_word_register_arg_transfer(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
        arg_bytes: u16,
    ) -> bool {
        if !self.can_forward_staged_word_arg_to_registers(arg, slot, arg_offset, arg_bytes) {
            return false;
        }

        if self.emit_staged_lvalue_word_register_arg_transfer(arg, arg_offset, arg_bytes) {
            return true;
        }

        if self.emit_staged_add_constant_indexed_word_register_arg_transfer(arg, arg_offset) {
            return true;
        }

        false
    }

    pub(super) fn emit_staged_lvalue_word_register_arg_transfer(
        &mut self,
        arg: &Expr,
        arg_offset: u8,
        arg_bytes: u16,
    ) -> bool {
        if arg_offset != 0 || (arg_bytes != 2 && arg_bytes != 3) {
            return false;
        }
        let Some(source) = self
            .reusable_lvalue_slot_with_pointer(arg, runtime_zp::ARRAY_ADDR)
            .or_else(|| self.dynamic_indexed_word_slot(arg))
        else {
            return false;
        };
        if source.size < 2 {
            return false;
        }

        self.emit_lda_slot_byte(source, 1);
        self.emit_tax();
        self.emit_lda_slot_byte(source, 0);
        self.record_modern_optimization(
            CodegenOptimizationKind::ArgumentStoreRemoved,
            6,
            Some(arg.span),
            "forwarded staged word lvalue directly into A/X",
        );
        true
    }

    pub(super) fn emit_staged_lvalue_word_to_x_a_with_late_constants(
        &mut self,
        arg: &Expr,
        spec: StagedCallArg<'_>,
        args: &StagedCallArgs<'_>,
        plan: &mut StagedCallRegisterPlan<'_>,
    ) -> bool {
        if spec.offset != 0 || spec.slot.size != 2 {
            return false;
        }
        let Some(source) = self
            .reusable_lvalue_slot_with_pointer(arg, runtime_zp::ARRAY_ADDR)
            .or_else(|| self.dynamic_indexed_word_slot(arg))
        else {
            return false;
        };
        if source.size < 2 {
            return false;
        }

        self.emit_lda_slot_byte(source, 1);
        self.emit_tax();
        for late in args.late_constant_byte_args() {
            if !self.emit_expr_to_slot(late.expr, late.slot) {
                return false;
            }
            plan.mark_staged(late.offset);
        }
        self.emit_lda_slot_byte(source, 0);
        plan.mark_preloaded_word(spec.offset);
        self.record_modern_optimization(
            CodegenOptimizationKind::ArgumentStoreRemoved,
            5,
            Some(arg.span),
            "forwarded staged word directly into A/X after staging late constants",
        );
        true
    }

    pub(super) fn emit_staged_lvalue_word_high_to_x_low_to_args(
        &mut self,
        arg: &Expr,
        arg_offset: u8,
        stack_low: bool,
    ) -> bool {
        if arg_offset != 0 {
            return false;
        }
        let Some(source) = self
            .reusable_lvalue_slot_with_pointer(arg, runtime_zp::ARRAY_ADDR)
            .or_else(|| self.dynamic_indexed_word_slot(arg))
        else {
            return false;
        };
        if source.size < 2 {
            return false;
        }

        self.emit_lda_slot_byte(source, 1);
        self.emit_tax();
        self.emit_lda_slot_byte(source, 0);
        if stack_low {
            self.emitter.emit_pha();
        } else {
            self.emit_sta_zero_page(runtime_zp::ARGS);
        }
        self.record_modern_optimization(
            if stack_low {
                CodegenOptimizationKind::ArgumentStackForwarded
            } else {
                CodegenOptimizationKind::ArgumentStoreRemoved
            },
            if stack_low { 5 } else { 3 },
            Some(arg.span),
            if stack_low {
                "forwarded staged word high byte directly into X and kept low byte on stack"
            } else {
                "forwarded staged word high byte directly into X and kept low byte in $A0"
            },
        );
        true
    }

    pub(super) fn emit_staged_add_constant_indexed_word_register_arg_transfer(
        &mut self,
        arg: &Expr,
        arg_offset: u8,
    ) -> bool {
        let ExprKind::Binary { op, left, right } = &arg.kind else {
            return false;
        };
        if *op != BinaryOp::Add {
            return false;
        }
        let (indexed, value) = if let Some(value) = self.constant_u16(right) {
            (left.as_ref(), value)
        } else if let Some(value) = self.constant_u16(left) {
            (right.as_ref(), value)
        } else {
            return false;
        };
        let Some(source) = self.dynamic_indexed_word_slot(indexed) else {
            return false;
        };
        let immediate = Immediate::new(value);

        match arg_offset {
            0 => {
                self.emit_clc();
                self.emit_lda_slot_byte(source, 0);
                self.emit_adc_immediate(immediate, 0);
                self.emitter.emit_pha();
                self.emit_lda_slot_byte(source, 1);
                self.emit_adc_immediate(immediate, 1);
                self.emit_tax();
                self.emit_pla();
                self.record_modern_optimization(
                    CodegenOptimizationKind::ArgumentStackForwarded,
                    5,
                    Some(arg.span),
                    "forwarded staged indexed word through stack directly into A/X",
                );
                true
            }
            1 => {
                self.emit_clc();
                self.emit_lda_slot_byte(source, 0);
                self.emit_adc_immediate(immediate, 0);
                self.emit_tax();
                self.emit_lda_slot_byte(source, 1);
                self.emit_adc_immediate(immediate, 1);
                self.emit_tay();
                self.straight_line_store_y = None;
                self.record_modern_optimization(
                    CodegenOptimizationKind::ArgumentStoreRemoved,
                    6,
                    Some(arg.span),
                    "forwarded staged indexed word bytes directly into X/Y",
                );
                true
            }
            _ => false,
        }
    }

    pub(super) fn emit_record_value_address_to_slot(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if slot.pointee_size.is_none() || slot.record.is_none() {
            return false;
        }
        let Some(address) = self.record_value_argument_base(arg, slot) else {
            return false;
        };
        self.emit_store_constant(slot, address.address());
        true
    }

    pub(super) fn emit_call_arguments_to_abi(
        &mut self,
        args: &[Expr],
        params: &[StorageSlot],
        literal_addresses: &[Option<Absolute>],
    ) -> bool {
        let mut register_args = Vec::new();
        let mut arg_offset = 0u8;
        let staged_first_word_pointer_arg = self.segment_storage
            && args.len() > 1
            && params.first().is_some_and(|slot| slot.size == 2)
            && first_arg_is_pointer_deref(&args[0])
            && self.expr_size(&args[0]) == Some(2)
            && self.emit_pointer_deref_word_arg_to_args(&args[0]);

        for ((arg, slot), literal_address) in args
            .iter()
            .zip(params.iter())
            .zip(literal_addresses.iter().copied())
        {
            if literal_address.is_none()
                && self.emit_indirect_word_call_arg_to_abi(arg, *slot, arg_offset)
            {
                arg_offset = arg_offset.wrapping_add(slot.size as u8);
                continue;
            }
            let byte_order: Vec<u16> = if self.segment_storage && arg_offset >= 3 && slot.size == 2
            {
                vec![1, 0]
            } else {
                (0..slot.size).collect()
            };
            for byte_index in byte_order {
                let target_offset = arg_offset.wrapping_add(byte_index as u8);
                if staged_first_word_pointer_arg && arg_offset == 0 && byte_index < 2 {
                    continue;
                }
                if target_offset < 3 {
                    register_args.push((target_offset, arg, *slot, byte_index, literal_address));
                } else {
                    if !self.emit_load_call_arg_byte(arg, *slot, byte_index, literal_address) {
                        return false;
                    }
                    self.emitter
                        .emit_sta_zero_page(runtime_zp::ARGS.offset(target_offset));
                }
            }
            arg_offset = arg_offset.wrapping_add(slot.size as u8);
        }

        for (arg_offset, arg, slot, byte_index, literal_address) in register_args.into_iter().rev()
        {
            if !self.emit_call_register_arg(arg_offset, arg, slot, byte_index, literal_address) {
                return false;
            }
        }
        if staged_first_word_pointer_arg {
            self.emit_ldx_zero_page(runtime_zp::ARGS.offset(1));
            self.emit_lda_zero_page(runtime_zp::ARGS);
        }

        true
    }

    pub(super) fn emit_indirect_word_call_arg_to_abi(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        arg_offset: u8,
    ) -> bool {
        if !self.segment_storage || slot.size != 2 || arg_offset < 3 {
            return false;
        }
        let Some(source) = self.reusable_lvalue_slot_with_pointer(arg, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if source.space != AddressSpace::IndirectIndexedY || source.size < 2 {
            return false;
        }

        self.emit_lda_slot_byte(source, 1);
        self.emit_sta_zero_page(runtime_zp::ARGS.offset(arg_offset.wrapping_add(1)));
        self.emit_dey();
        self.emit_lda_slot_byte(source, 0);
        self.emit_sta_zero_page(runtime_zp::ARGS.offset(arg_offset));
        true
    }

    pub(super) fn emit_pointer_deref_word_arg_to_args(&mut self, arg: &Expr) -> bool {
        let Some(source) = self.reusable_lvalue_slot_with_pointer(arg, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if source.space != AddressSpace::IndirectIndexedY || source.size < 2 {
            return false;
        }
        self.emit_lda_slot_byte(source, 1);
        self.emit_sta_zero_page(runtime_zp::ARGS.offset(1));
        self.emit_lda_slot_byte(source, 0);
        self.emit_sta_zero_page(runtime_zp::ARGS);
        true
    }

    pub(super) fn emit_call_register_arg(
        &mut self,
        arg_offset: u8,
        arg: &Expr,
        slot: StorageSlot,
        byte_index: u16,
        literal_address: Option<Absolute>,
    ) -> bool {
        debug_assert_call_arg_byte_shape(slot, arg_offset, byte_index);
        match arg_offset {
            0 => self.emit_load_call_arg_byte(arg, slot, byte_index, literal_address),
            1 => {
                if let Some(byte) =
                    self.call_arg_immediate_byte(arg, slot, byte_index, literal_address)
                {
                    self.emit_ldx_imm(byte);
                    true
                } else if let Some(zero_page) = self.call_arg_zero_page_byte(arg, byte_index) {
                    self.emit_ldx_zero_page(zero_page);
                    true
                } else if let Some(absolute) = self.call_arg_absolute_byte(arg, byte_index) {
                    self.emit_ldx_absolute(absolute);
                    true
                } else if self.emit_load_call_arg_byte(arg, slot, byte_index, literal_address) {
                    self.emit_tax();
                    true
                } else {
                    false
                }
            }
            2 => {
                if let Some(byte) =
                    self.call_arg_immediate_byte(arg, slot, byte_index, literal_address)
                {
                    self.emit_ldy_imm(byte);
                    self.straight_line_store_y = None;
                    true
                } else if let Some(zero_page) = self.call_arg_zero_page_byte(arg, byte_index) {
                    self.emit_ldy_slot_byte(StorageSlot::zero_page(zero_page.address(), 1), 0);
                    self.straight_line_store_y = None;
                    true
                } else if let Some(absolute) = self.call_arg_absolute_byte(arg, byte_index) {
                    self.emit_ldy_slot_byte(StorageSlot::absolute(absolute.address(), 1), 0);
                    self.straight_line_store_y = None;
                    true
                } else if self.emit_load_call_arg_byte(arg, slot, byte_index, literal_address) {
                    self.emit_tay();
                    self.straight_line_store_y = None;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub(super) fn call_arg_zero_page_byte(&self, arg: &Expr, byte_index: u16) -> Option<ZeroPage> {
        let slot = self.call_arg_direct_slot(arg)?;
        let address = slot.byte_address(byte_index);
        if byte_index < slot.size
            && (slot.space == AddressSpace::ZeroPage
                || (slot.space == AddressSpace::Absolute && address < 0x100))
        {
            Some(slot.zero_page_byte(byte_index))
        } else {
            None
        }
    }

    pub(super) fn call_arg_absolute_byte(&self, arg: &Expr, byte_index: u16) -> Option<Absolute> {
        let slot = self.call_arg_direct_slot(arg)?;
        if matches!(
            slot.array,
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor)
        ) && byte_index < 2
        {
            return Some(Absolute::new(slot.address.wrapping_add(byte_index)));
        }
        if slot.space == AddressSpace::Absolute && byte_index < slot.size {
            Some(slot.absolute_byte(byte_index))
        } else {
            None
        }
    }

    pub(super) fn call_arg_direct_slot(&self, arg: &Expr) -> Option<StorageSlot> {
        let slot = match &arg.kind {
            ExprKind::Name(name) => self.lookup_slot(name)?,
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let base_slot = self.lookup_slot(name)?;
                if base_slot.pointee_size.is_some() || base_slot.space != AddressSpace::Absolute {
                    return None;
                }
                let field = self.record_layouts.field(base_slot.record?, field)?;
                StorageSlot::absolute(base_slot.address.wrapping_add(field.offset), field.size)
            }
            _ => return None,
        };
        Some(slot)
    }

    pub(super) fn call_arg_immediate_byte(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        byte_index: u16,
        literal_address: Option<Absolute>,
    ) -> Option<u8> {
        if let Some(value) = self.constant_u16(arg) {
            return Some(Immediate::new(value).byte(byte_index));
        }
        if let Some(address) = literal_address {
            return Some(Immediate::new(address.address()).byte(byte_index));
        }
        if let Some(address) = self.array_argument_base(arg) {
            return Some(Immediate::new(address.address()).byte(byte_index));
        }
        if let Some(address) = self.record_value_argument_base(arg, slot) {
            return Some(Immediate::new(address.address()).byte(byte_index));
        }
        if let ExprKind::Unary {
            op: UnaryOp::AddressOf,
            expr,
        } = &arg.kind
        {
            let address = self.address_of_lvalue(expr)?;
            return Some(Immediate::new(address.address()).byte(byte_index));
        }
        None
    }

    pub(super) fn emit_load_call_arg_byte(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        byte_index: u16,
        literal_address: Option<Absolute>,
    ) -> bool {
        debug_assert_call_arg_value_shape(arg, slot, byte_index, literal_address);
        if let Some(byte) = self.call_arg_immediate_byte(arg, slot, byte_index, literal_address) {
            self.emit_lda_imm(byte);
            return true;
        }
        if self.emit_load_routine_address_arg_byte(arg, slot, byte_index) {
            return true;
        }
        if self.emit_load_array_descriptor_pointer_byte(arg, byte_index) {
            return true;
        }
        if byte_index == 0
            && self.can_emit_modern_first_byte_arg_expr_to_acc(arg, slot)
            && self.emit_modern_byte_expr_to_acc(arg)
        {
            return true;
        }
        if byte_index < slot.size
            && slot.size <= 2
            && self.emit_load_effective_address_byte(arg, byte_index)
        {
            return true;
        }
        if self.emit_inline_byte_array_scalar_index_load(arg, byte_index) {
            return true;
        }
        if let Some(source) = self.lvalue_slot(arg) {
            if byte_index >= source.size {
                self.emit_lda_imm(0);
            } else {
                if self.can_forward_recent_a_store(source, byte_index) {
                    self.record_modern_optimization(
                        CodegenOptimizationKind::ArgumentStoreRemoved,
                        slot_load_instruction_len(source),
                        Some(arg.span),
                        "forwarded recently stored accumulator into call argument",
                    );
                    return true;
                }
                self.emit_lda_slot_byte_value_only(source, byte_index);
            }
            return true;
        }
        self.emit_load_simple_byte(arg, byte_index)
    }

    pub(super) fn emit_modern_byte_expr_to_acc(&mut self, arg: &Expr) -> bool {
        match &arg.kind {
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor),
                left,
                right,
            } => self.emit_binary_expr_byte(*op, left, right, 0, true),
            ExprKind::Binary {
                op: op @ (BinaryOp::Lsh | BinaryOp::Rsh),
                left,
                right,
            } => {
                let Some(count) = self.constant_u16(right) else {
                    return false;
                };
                if count >= 8 {
                    self.emit_lda_imm(0);
                    true
                } else {
                    self.emit_byte_constant_shift_to_acc(*op, left, count)
                }
            }
            _ => false,
        }
    }

    pub(super) fn emit_load_routine_address_arg_byte(
        &mut self,
        arg: &Expr,
        slot: StorageSlot,
        byte_index: u16,
    ) -> bool {
        if slot.size < 2 || byte_index > 1 {
            return false;
        }
        let ExprKind::Name(name) = &arg.kind else {
            return false;
        };
        let Some(routine) = self.routines.get(&normalize_name(name)).cloned() else {
            return false;
        };
        if let Some(address) = routine.system_address {
            self.emitter
                .emit_lda_immediate(Immediate::new(address), byte_index);
        } else if byte_index == 0 {
            self.emit_lda_label_low(routine.label, arg.span);
        } else {
            self.emit_lda_label_high(routine.label, arg.span);
        }
        true
    }

    pub(super) fn emit_load_staged_call_registers(
        &mut self,
        arg_bytes: u16,
        plan: &StagedCallRegisterPlan<'_>,
    ) {
        if arg_bytes == 2
            && let Some(deferred) = plan.deferred_direct_word_at(0)
            && self.can_forward_staged_word_arg_to_registers(
                deferred.expr,
                deferred.slot,
                deferred.offset,
                arg_bytes,
            )
            && self.emit_staged_lvalue_word_register_arg_transfer(
                deferred.expr,
                deferred.offset,
                arg_bytes,
            )
        {
            return;
        }
        if arg_bytes > 2 {
            if plan.is_preloaded(2) {
                self.straight_line_store_y = None;
            } else if let Some(deferred) = plan.deferred_at(2) {
                self.emit_deferred_call_register_arg(2, deferred, 0);
            } else if let Some(deferred) = plan.deferred_word_at(1) {
                self.emit_deferred_call_register_arg(2, deferred, 1);
            } else {
                self.emit_raw_ldy_slot_byte(
                    StorageSlot::zero_page(runtime_zp::ARGS.offset(2).address(), 1),
                    0,
                );
                self.straight_line_store_y = None;
            }
        }
        if arg_bytes > 1 {
            if plan.is_preloaded(1) {
                // Already transferred from A to X while preserving source evaluation order.
            } else if let Some(deferred) = plan.deferred_at(1) {
                self.emit_deferred_call_register_arg(1, deferred, 0);
            } else if let Some(deferred) = plan.deferred_word_at(0) {
                self.emit_deferred_call_register_arg(1, deferred, 1);
            } else {
                self.emit_ldx_slot_byte(
                    StorageSlot::zero_page(runtime_zp::ARGS.offset(1).address(), 1),
                    0,
                );
            }
        }
        if arg_bytes > 0 {
            if plan.is_preloaded(0) {
                // Already loaded into A while preserving source evaluation order.
            } else if let Some(deferred) = plan.deferred_at(0) {
                self.emit_deferred_call_register_arg(0, deferred, 0);
            } else if plan.is_stacked(0) {
                self.emit_pla();
            } else if self.rewrite_zero_extended_staged_first_arg_to_stack() {
                // Rewritten suffix leaves the low byte in A and high byte zero in X.
            } else {
                self.emit_lda_zero_page(runtime_zp::ARGS);
            }
        }
    }

    pub(super) fn rewrite_zero_extended_staged_first_arg_to_stack(&mut self) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        let suffix = [
            opcode::STA_ZP,
            runtime_zp::ARGS.address(),
            opcode::LDA_IMM,
            0,
            opcode::STA_ZP,
            runtime_zp::ARGS.offset(1).address(),
            opcode::TAX,
        ];
        let len = self.emitter.bytes.len();
        let start = len.saturating_sub(suffix.len());
        if self.emitter.bytes.get(start..len) != Some(&suffix) {
            return false;
        }

        self.delete_emitted_bytes(start, suffix.len());
        self.processor.reset();
        self.emit_ldx_imm(0);
        self.record_modern_optimization(
            CodegenOptimizationKind::ArgumentStackForwarded,
            5,
            None,
            "forwarded zero-extended staged first argument directly into A/X",
        );
        true
    }

    pub(super) fn emit_deferred_call_register_arg(
        &mut self,
        register_offset: u8,
        deferred: DeferredCallRegisterArg<'_>,
        byte_index: u16,
    ) {
        self.emit_call_register_arg(
            register_offset,
            deferred.expr,
            deferred.slot,
            byte_index,
            deferred.literal_address,
        );
    }
}

impl Generator {
    pub(super) fn call_target_effects(&self, callee: &Expr) -> Option<RoutineEffects> {
        let ExprKind::Name(name) = &callee.kind else {
            return None;
        };
        self.routines
            .get(&normalize_name(name))
            .map(|info| info.effects)
    }

    pub(super) fn call_expr_preserves_slot_byte(
        &self,
        expr: &Expr,
        slot: StorageSlot,
        byte_index: u16,
    ) -> bool {
        let ExprKind::Call { callee, args } = &expr.kind else {
            return false;
        };
        if self.array_call_slot_size(callee, args).is_some()
            || args
                .iter()
                .any(|arg| expr_contains_routine_call(arg, &self.routines))
        {
            return false;
        }
        self.call_target_effects(callee).is_some_and(|effects| {
            effects.known
                && match slot.space {
                    AddressSpace::ZeroPage => {
                        !effects.writes_zero_page(slot.zero_page_byte(byte_index))
                    }
                    AddressSpace::Absolute => {
                        !effects.writes_absolute_range(slot.byte_address(byte_index), 1)
                    }
                    AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => false,
                }
        })
    }

    pub(super) fn expr_preserves_call_return_slot(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Call { callee, args } => self.array_call_slot_size(callee, args).is_some(),
            ExprKind::Binary { .. } => false,
            ExprKind::Unary {
                op: UnaryOp::Neg, ..
            } => false,
            ExprKind::Unary { expr, .. } => self.expr_preserves_call_return_slot(expr),
            _ => true,
        }
    }

    pub(super) fn emit_accumulator_call_return_to_slot(
        &mut self,
        source: StorageSlot,
        target: StorageSlot,
        internal_abi: &RoutineInternalAbi,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        debug_assert_eq!(internal_abi.public_result_slot(), Some(source));

        if internal_abi.result_byte_is_register_a(0) && source.size >= 1 && target.size >= 1 {
            if target.size == 1 {
                self.emit_sta_slot_byte(target, 0);
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallResultMaterializationRemoved,
                    slot_load_instruction_len(source),
                    None,
                    "stored byte return directly from accumulator",
                );
                return true;
            }
            if source.size == 1 && target.size == 2 {
                self.emit_sta_slot_byte(target, 0);
                self.emit_lda_imm(0);
                self.emit_sta_slot_byte(target, 1);
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallResultMaterializationRemoved,
                    slot_load_instruction_len(source),
                    None,
                    "stored zero-extended byte return directly from accumulator",
                );
                return true;
            }
            if self.segment_storage && source.size == 2 && target.size == 2 {
                self.emit_sta_slot_byte(target, 0);
                self.emit_lda_slot_byte(source, 1);
                self.emit_sta_slot_byte(target, 1);
                self.record_modern_optimization(
                    CodegenOptimizationKind::CallResultMaterializationRemoved,
                    slot_load_instruction_len(source),
                    None,
                    "stored low byte of word return directly from accumulator",
                );
                return true;
            }
        }
        if self.profile.enables_modern_optimizations()
            && self.segment_storage
            && source.size == 2
            && target.size == 2
            && internal_abi.result_byte_is_register_a(1)
        {
            self.emit_sta_slot_byte(target, 1);
            self.emit_lda_slot_byte(source, 0);
            self.emit_sta_slot_byte(target, 0);
            self.record_modern_optimization(
                CodegenOptimizationKind::CallResultMaterializationRemoved,
                slot_load_instruction_len(source),
                None,
                "stored high byte of word return directly from accumulator",
            );
            return true;
        }

        false
    }

    pub(super) fn emit_copy_call_return_slot_to_slot(
        &mut self,
        source: StorageSlot,
        target: StorageSlot,
        internal_abi: RoutineInternalAbi,
    ) -> bool {
        debug_assert_copy_slot_shape(source, target);
        if source == target {
            return true;
        }
        debug_assert_indirect_slots_do_not_alias(source, target, "call return copy");
        if self.emit_accumulator_call_return_to_slot(source, target, &internal_abi) {
            return true;
        }

        self.emit_copy_slot_to_slot(source, target)
    }
}

impl Generator {
    pub(super) fn generate_call_stmt(&mut self, expr: &Expr, span: Span) {
        let ExprKind::Call { callee, args } = &expr.kind else {
            if let ExprKind::Name(name) = &expr.kind
                && let Some(items) = self.machine_defines.get(&normalize_name(name)).cloned()
            {
                self.emit_machine_block(&items, span);
                return;
            }
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen only supports routine call statements",
            ));
            return;
        };
        if !self.emit_call(callee, args, span) {
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen only supports user routine calls and numeric-address system calls",
            ));
        }
    }

    pub(super) fn emit_routine_parameter_prologue(&mut self, routine: &Routine) {
        if let Some((frame_base, arg_bytes)) = self.routine_sargs_frame(routine) {
            debug_assert_sargs_helper_abi(
                &self.runtime_helpers.target(RuntimeHelperSlot::SArgs),
                frame_base,
                arg_bytes,
            );
            self.emit_jsr_runtime_helper(
                RuntimeHelperSlot::SArgs,
                self.runtime_helpers.target(RuntimeHelperSlot::SArgs),
                routine.span,
            );
            let frame_base = Absolute::new(frame_base);
            self.emitter.emit_u8(frame_base.low());
            self.emitter.emit_u8(frame_base.high());
            self.emitter.emit_u8(arg_bytes.wrapping_sub(1));
            return;
        }

        let mut bindings = Vec::new();
        let mut arg_offset = 0u8;
        for param in &routine.params {
            let Some(slot_size) = self.routine_param_slot_size(param) else {
                continue;
            };
            for entry in &param.entries {
                let Some(slot) = self
                    .local_symbols
                    .get(&normalize_name(&entry.name))
                    .copied()
                else {
                    arg_offset = arg_offset.wrapping_add(slot_size as u8);
                    continue;
                };
                for byte_index in 0..slot_size {
                    bindings.push((arg_offset, slot, byte_index));
                    arg_offset = arg_offset.wrapping_add(1);
                }
            }
        }

        if self.segment_storage && bindings.len() == 2 {
            for (arg_offset, slot, byte_index) in bindings.into_iter().rev() {
                self.emit_store_call_abi_byte_to_slot(arg_offset, slot, byte_index);
            }
        } else {
            for (arg_offset, slot, byte_index) in bindings {
                self.emit_store_call_abi_byte_to_slot(arg_offset, slot, byte_index);
            }
        }
    }

    pub(super) fn routine_sargs_frame(&self, routine: &Routine) -> Option<(u16, u8)> {
        if !self.segment_storage {
            return None;
        }

        let mut frame_base = None;
        let mut arg_bytes = 0u16;
        for param in &routine.params {
            let slot_size = self.routine_param_slot_size(param)?;
            for entry in &param.entries {
                let slot = self
                    .local_symbols
                    .get(&normalize_name(&entry.name))
                    .copied()?;
                frame_base.get_or_insert(slot.address);
                arg_bytes = arg_bytes.wrapping_add(slot_size);
            }
        }

        if arg_bytes < 3 || arg_bytes > u16::from(u8::MAX) + 1 {
            return None;
        }

        Some((frame_base?, arg_bytes as u8))
    }

    pub(super) fn routine_param_slot_size(&self, param: &VarDecl) -> Option<u16> {
        if decl_is_array_like(param) {
            Some(2)
        } else {
            storage_size_with_records(&param.ty, &self.record_layouts)
        }
    }

    pub(super) fn emit_store_call_abi_byte_to_slot(
        &mut self,
        arg_offset: u8,
        slot: StorageSlot,
        byte_index: u16,
    ) {
        match arg_offset {
            0 => self.emit_sta_slot_byte(slot, byte_index),
            1 => self.emit_stx_slot_byte(slot, byte_index),
            2 => self.emit_sty_slot_byte(slot, byte_index),
            _ => {
                self.emit_lda_zero_page(runtime_zp::ARGS.offset(arg_offset));
                self.emit_sta_slot_byte(slot, byte_index);
            }
        }
    }
}
