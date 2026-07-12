use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForStep {
    Up(u16),
    Down(u16),
}

enum ForEndCache {
    ByteLabel(String),
    WordLabels { low: String, high: String },
    ByteSlot(StorageSlot),
    WordSlots { low: StorageSlot, high: StorageSlot },
}

impl ForStep {
    fn amount(self) -> u16 {
        match self {
            Self::Up(amount) | Self::Down(amount) => amount,
        }
    }
}

fn constant_for_step(expr: &Expr) -> Option<ForStep> {
    match &expr.kind {
        ExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } => Some(ForStep::Down(constant_u16(expr)?)),
        _ => Some(ForStep::Up(constant_u16(expr)?)),
    }
}

impl Generator {
    pub(super) fn generate_stmt(&mut self, stmt: &Stmt) {
        let start = self.current_absolute_address();
        match stmt {
            Stmt::Define(define) => self.generate_define(define),
            Stmt::Return(expr) => self.generate_return(expr.as_ref()),
            Stmt::Assign {
                target,
                value,
                span,
            } => self.generate_assignment(target, value, *span),
            Stmt::CompoundAssign {
                target,
                op,
                value,
                span,
            } => self.generate_compound_assignment(target, *op, value, *span),
            Stmt::If {
                branches,
                else_body,
                span,
            } => self.generate_if(branches, else_body, *span),
            Stmt::While {
                condition,
                body,
                span,
            } => self.generate_while(condition, body, *span),
            Stmt::DoUntil {
                body,
                condition,
                span,
            } => self.generate_do_until(body, condition.as_ref(), *span),
            Stmt::For {
                target,
                start,
                end,
                step,
                body,
                span,
            } => self.generate_for(target, start, end, step.as_ref(), body, *span),
            Stmt::Exit { span } => self.generate_exit(*span),
            Stmt::Call { expr, span } => self.generate_call_stmt(expr, *span),
            Stmt::MachineBlock { items, span, .. } => {
                self.record_machine_block_analysis(items, *span, start);
                if !self.current_routine_has_effect_contract {
                    self.record_current_unknown_effects();
                }
                self.emit_machine_block(items, *span);
            }
            _ => self.diagnostics.push(Diagnostic::new(
                stmt_span(stmt),
                "codegen for this statement is not implemented yet",
            )),
        }
        self.record_source_range(
            stmt_source_range_kind(stmt),
            Some(stmt_source_range_name(stmt).to_string()),
            stmt_span(stmt),
            start,
            self.current_absolute_address(),
        );
    }

    pub(super) fn generate_define(&mut self, define: &DefineDecl) {
        for entry in &define.entries {
            if let Some(items) = parse_machine_define_value(&entry.value) {
                self.machine_defines
                    .insert(normalize_name(&entry.name), items);
            }
        }
    }

    pub(super) fn generate_stmt_list(&mut self, body: &[Stmt]) {
        let previous_lookahead = self.y_constant_store_lookahead;
        let mut index = 0usize;
        while index < body.len() {
            let stmt = &body[index];
            self.y_constant_store_lookahead =
                self.next_y_constant_store_in_straight_line(&body[index + 1..]);
            if self.emit_modern_tail_call_stmt(stmt, body.get(index + 1)) {
                index += 2;
                continue;
            }
            if self.emit_modern_pointer_compound_return(stmt, body.get(index + 1)) {
                index += 2;
                continue;
            }
            self.generate_stmt(stmt);
            index += 1;
        }
        self.y_constant_store_lookahead = previous_lookahead;
    }

    pub(super) fn emit_modern_pointer_compound_return(
        &mut self,
        stmt: &Stmt,
        next: Option<&Stmt>,
    ) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        let (target, offset, span) = match stmt {
            Stmt::CompoundAssign {
                target,
                op: BinaryOp::Add,
                value,
                span,
            } => {
                let ExprKind::Name(name) = &target.kind else {
                    return false;
                };
                let Some(offset) = self.pointer_deref_plus_constant_for_name(value, name, 0) else {
                    return false;
                };
                (target, offset, *span)
            }
            Stmt::Assign {
                target,
                value,
                span,
            } => {
                let ExprKind::Name(name) = &target.kind else {
                    return false;
                };
                let Some(offset) =
                    self.expanded_pointer_compound_add_offset_for_name(target, value, name)
                else {
                    return false;
                };
                (target, offset, *span)
            }
            _ => return false,
        };
        let Some(Stmt::Return(Some(return_expr))) = next else {
            return false;
        };
        let ExprKind::Name(name) = &target.kind else {
            return false;
        };
        let Some(pointer) = self.lookup_slot(name) else {
            return false;
        };
        if pointer.size != 2 || pointer.pointee_size != Some(1) {
            return false;
        }
        if !expr_is_pointer_deref_name(return_expr, name) {
            return false;
        }
        let Some(return_slot) = self.current_return_slot else {
            return false;
        };
        if return_slot.size != 1
            || return_slot.space != AddressSpace::ZeroPage
            || return_slot.address != u16::from(runtime_zp::ARGS.address())
        {
            return false;
        }

        let compound_start = self.current_absolute_address();
        if !self.emit_pointer_slot_to_addr(pointer, runtime_zp::ARRAY_ADDR) {
            return false;
        }
        let deref = StorageSlot::indirect_indexed_y(runtime_zp::ARRAY_ADDR, 1);
        self.emit_clc();
        self.ensure_y_imm(0);
        self.emit_lda_slot_byte(deref, 0);
        self.emit_adc_immediate(Immediate::new(offset), 0);
        self.emit_sta_zero_page(runtime_zp::ELEMENT_ADDR);
        self.emit_tya();
        self.emit_sta_zero_page(runtime_zp::ELEMENT_ADDR.offset(1));

        self.emit_clc();
        self.emit_lda_zero_page(runtime_zp::ELEMENT_ADDR);
        self.emit_adc_slot_byte(pointer, 0);
        self.emit_sta_slot_byte(pointer, 0);
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);
        self.emit_lda_zero_page(runtime_zp::ELEMENT_ADDR.offset(1));
        self.emit_adc_slot_byte(pointer, 1);
        self.emit_sta_slot_byte(pointer, 1);
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        self.record_source_range(
            stmt_source_range_kind(stmt),
            Some(stmt_source_range_name(stmt).to_string()),
            span,
            compound_start,
            self.current_absolute_address(),
        );

        let return_start = self.current_absolute_address();
        self.emit_lda_slot_byte(deref, 0);
        self.emit_sta_slot_byte(return_slot, 0);
        self.record_inferred_return_facts(return_slot);
        self.emit_return_rts(stmt_span(next.expect("return statement")));
        self.record_source_range(
            stmt_source_range_kind(next.expect("return statement")),
            Some(stmt_source_range_name(next.expect("return statement")).to_string()),
            stmt_span(next.expect("return statement")),
            return_start,
            self.current_absolute_address(),
        );
        true
    }

    pub(super) fn expanded_pointer_compound_add_offset_for_name(
        &self,
        target: &Expr,
        value: &Expr,
        name: &str,
    ) -> Option<u16> {
        let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = &value.kind
        else {
            return None;
        };
        if Self::same_lvalue_expr(target, left) {
            return self.pointer_deref_plus_constant_for_name(right, name, 0);
        }
        if Self::same_lvalue_expr(target, right) {
            return self.pointer_deref_plus_constant_for_name(left, name, 0);
        }
        if let Some(offset) = self.expanded_pointer_compound_add_offset_for_name(target, left, name)
        {
            let constant = self.constant_u16(right)?;
            return Some(offset.wrapping_add(constant));
        }
        if let Some(offset) =
            self.expanded_pointer_compound_add_offset_for_name(target, right, name)
        {
            let constant = self.constant_u16(left)?;
            return Some(offset.wrapping_add(constant));
        }
        None
    }

    pub(super) fn pointer_deref_plus_constant_for_name(
        &self,
        expr: &Expr,
        name: &str,
        offset: u16,
    ) -> Option<u16> {
        if expr_is_pointer_deref_name(expr, name) {
            return Some(offset);
        }
        let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = &expr.kind
        else {
            return None;
        };
        if let Some(value) = self.constant_u16(right) {
            return self.pointer_deref_plus_constant_for_name(
                left,
                name,
                offset.wrapping_add(value),
            );
        }
        if let Some(value) = self.constant_u16(left) {
            return self.pointer_deref_plus_constant_for_name(
                right,
                name,
                offset.wrapping_add(value),
            );
        }
        None
    }

    fn next_y_constant_store_in_straight_line(&self, body: &[Stmt]) -> Option<u8> {
        for stmt in body {
            match stmt {
                Stmt::Define(_) => continue,
                Stmt::Assign { target, value, .. } => {
                    if let Some(value) = self.y_constant_store_value(target, value) {
                        return Some(value);
                    }
                    if self.assignment_is_simple_constant_store(target, value) {
                        continue;
                    }
                    return None;
                }
                Stmt::If {
                    branches,
                    else_body,
                    ..
                } => {
                    for branch in branches {
                        if let Some(value) =
                            self.next_y_constant_store_in_straight_line(&branch.body)
                        {
                            return Some(value);
                        }
                    }
                    if let Some(value) = self.next_y_constant_store_in_straight_line(else_body) {
                        return Some(value);
                    }
                    return None;
                }
                Stmt::CompoundAssign { .. }
                | Stmt::Call { .. }
                | Stmt::MachineBlock { .. }
                | Stmt::While { .. }
                | Stmt::DoUntil { .. }
                | Stmt::For { .. }
                | Stmt::Exit { .. }
                | Stmt::Return(_)
                | Stmt::Unsupported { .. } => return None,
            }
        }
        None
    }

    fn y_constant_store_value(&self, target: &Expr, value: &Expr) -> Option<u8> {
        let value = self.constant_u16(value)?;
        if !matches!(value, 0 | 1) {
            return None;
        }
        let slot = self.peek_lvalue_slot(target)?;
        (slot.size <= 2 && self.compatible_y_constant_store_slot(slot)).then_some(value as u8)
    }

    fn assignment_is_simple_constant_store(&self, target: &Expr, value: &Expr) -> bool {
        self.constant_u16(value).is_some()
            && self.peek_lvalue_slot(target).is_some_and(|slot| {
                matches!(slot.space, AddressSpace::Absolute | AddressSpace::ZeroPage)
            })
    }

    pub(super) fn generate_exit(&mut self, span: Span) {
        let Some(label) = self.exit_labels.last() else {
            self.diagnostics
                .push(Diagnostic::new(span, "EXIT is not inside a loop"));
            return;
        };
        self.emit_jmp_label(label.clone(), span);
    }

    pub(super) fn generate_return(&mut self, expr: Option<&Expr>) {
        if let Some(expr) = expr {
            let Some(slot) = self.current_return_slot else {
                self.diagnostics.push(Diagnostic::new(
                    expr.span,
                    "procedure RETURN cannot include a value",
                ));
                return;
            };
            if !self.emit_expr_to_slot(expr, slot) {
                self.diagnostics.push(Diagnostic::new(
                    expr.span,
                    "codegen only supports scalar function return values",
                ));
                return;
            }
            self.record_inferred_return_facts(slot);
        } else if let Some(facts) = &mut self.current_inferred_routine_facts {
            facts.returns_a_equals_a0_candidate = false;
            facts.returns_a_equals_a1_candidate = false;
        }
        self.emit_return_rts(expr.map_or(Span::new(0, 0), |expr| expr.span));
    }

    pub(super) fn emit_return_rts(&mut self, span: Span) {
        self.rewrite_jumps_to_current_rts(span);
        if self.try_rewrite_jsr_rts_to_tail_jmp(span) {
            return;
        }
        self.emitter.emit_rts();
    }

    pub(super) fn generate_if(&mut self, branches: &[IfBranch], else_body: &[Stmt], span: Span) {
        let end_label = self.next_label("if:end");

        for (index, branch) in branches.iter().enumerate() {
            let next_label = self.next_label("if:next");
            if !self.emit_branch_if_false(&branch.condition, &next_label, span) {
                self.diagnostics.push(Diagnostic::new(
                    branch.condition.span,
                    "codegen only supports scalar IF conditions and unsigned comparisons",
                ));
                return;
            }
            self.generate_stmt_list(&branch.body);
            if (index + 1 < branches.len() || !else_body.is_empty())
                && !(self.profile.enables_modern_optimizations()
                    && stmt_list_ends_with_terminal_flow(&branch.body))
            {
                self.emit_jmp_label(&end_label, span);
            }
            self.bind_codegen_label(next_label, span);
        }

        self.generate_stmt_list(else_body);
        self.bind_codegen_label(end_label, span);
    }

    pub(super) fn generate_while(&mut self, condition: &Expr, body: &[Stmt], span: Span) {
        let start_label = self.next_label("while:start");
        let end_label = self.next_label("while:end");

        self.bind_codegen_label(start_label.clone(), span);
        if !self.emit_branch_if_false(condition, &end_label, span) {
            self.diagnostics.push(Diagnostic::new(
                condition.span,
                "codegen only supports scalar WHILE conditions and unsigned comparisons",
            ));
            return;
        }
        self.exit_labels.push(end_label.clone());
        self.generate_stmt_list(body);
        self.exit_labels.pop();
        self.emit_jmp_label(start_label, span);
        if !self.profile.enables_modern_optimizations() {
            self.label_store_y_hints.remove(&end_label);
        }
        self.bind_codegen_label(end_label, span);
    }

    pub(super) fn generate_do_until(
        &mut self,
        body: &[Stmt],
        condition: Option<&Expr>,
        span: Span,
    ) {
        let start_label = self.next_label("do:start");
        let end_label = self.next_label("do:end");

        self.bind_codegen_label(start_label.clone(), span);
        self.exit_labels.push(end_label.clone());
        self.generate_stmt_list(body);
        self.exit_labels.pop();

        if let Some(condition) = condition {
            if !self.emit_branch_if_false(condition, &start_label, span) {
                self.diagnostics.push(Diagnostic::new(
                    condition.span,
                    "codegen only supports scalar UNTIL conditions and unsigned comparisons",
                ));
                return;
            }
        } else {
            self.emit_jmp_label(start_label, span);
        }
        self.bind_codegen_label(end_label, span);
    }

    pub(super) fn generate_for(
        &mut self,
        target: &Expr,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        span: Span,
    ) {
        let Some(step) = step.map_or(Some(ForStep::Up(1)), constant_for_step) else {
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen only supports constant FOR steps",
            ));
            return;
        };
        if step.amount() == 0 {
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen only supports non-zero FOR steps",
            ));
            return;
        }

        self.generate_assignment(target, start, span);

        let ExprKind::Name(name) = &target.kind else {
            self.diagnostics.push(Diagnostic::new(
                target.span,
                "codegen only supports simple variable FOR targets",
            ));
            return;
        };
        let Some(slot) = self.lookup_slot(name) else {
            self.diagnostics.push(Diagnostic::new(
                target.span,
                format!("no storage allocated for `{name}`"),
            ));
            return;
        };

        let start_label = self.next_label("for:start");
        let end_label = self.next_label("for:end");
        let cached_end = self.emit_compatible_for_end_cache(end, slot, step, span);

        self.bind_codegen_label(start_label.clone(), span);
        if let Some(cached_end) = cached_end {
            let body_label = self.next_label("for:body");
            match cached_end {
                ForEndCache::ByteLabel(cache_label) => {
                    self.emit_lda_absolute_label(cache_label.clone(), span);
                    self.emit_cmp_slot_byte(slot, 0);
                    self.emitter
                        .emit_branch_label(opcode::BCS_REL, &body_label, span);
                    self.emit_jmp_label(&end_label, span);
                    self.bind_codegen_label(cache_label, span);
                    self.emitter.emit_u8(0);
                }
                ForEndCache::WordLabels { low, high } => {
                    self.emit_lda_absolute_label(low.clone(), span);
                    self.emit_cmp_slot_byte(slot, 0);
                    self.emit_lda_absolute_label(high.clone(), span);
                    self.emit_sbc_slot_byte(slot, 1);
                    self.emitter
                        .emit_branch_label(opcode::BCS_REL, &body_label, span);
                    self.emit_jmp_label(&end_label, span);
                    self.bind_codegen_label(low, span);
                    self.emitter.emit_u8(0);
                    self.bind_codegen_label(high, span);
                    self.emitter.emit_u8(0);
                }
                ForEndCache::ByteSlot(cache) => {
                    self.emit_lda_slot_byte(cache, 0);
                    self.emit_cmp_slot_byte(slot, 0);
                    self.emitter
                        .emit_branch_label(opcode::BCS_REL, &body_label, span);
                    self.emit_jmp_label(&end_label, span);
                }
                ForEndCache::WordSlots { low, high } => {
                    self.emit_lda_slot_byte(low, 0);
                    self.emit_cmp_slot_byte(slot, 0);
                    self.emit_lda_slot_byte(high, 0);
                    self.emit_sbc_slot_byte(slot, 1);
                    self.emitter
                        .emit_branch_label(opcode::BCS_REL, &body_label, span);
                    self.emit_jmp_label(&end_label, span);
                }
            }
            self.bind_codegen_label(body_label, span);
        } else {
            let compare = match step {
                ForStep::Up(_) => BinaryOp::Le,
                ForStep::Down(_) if self.segment_storage => BinaryOp::Le,
                ForStep::Down(_) => BinaryOp::Ge,
            };
            if !self.emit_branch_if_false_compare(compare, target, end, &end_label, span) {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    "codegen only supports scalar unsigned FOR bounds",
                ));
                return;
            }
        }

        self.exit_labels.push(end_label.clone());
        self.generate_stmt_list(body);
        self.exit_labels.pop();
        self.emit_for_step_slot(slot, step);
        self.emit_jmp_label(start_label, span);
        self.bind_codegen_label(end_label, span);
    }

    fn emit_compatible_for_end_cache(
        &mut self,
        end: &Expr,
        target: StorageSlot,
        step: ForStep,
        span: Span,
    ) -> Option<ForEndCache> {
        if (self.profile.enables_modern_optimizations() || self.preserve_modern_routine_layout)
            && let Some(cache) = self
                .current_modern_routine_layout
                .for_end_caches
                .get(&SpanKey::from(span))
                .copied()
        {
            return self.emit_modern_for_end_cache(end, cache, span);
        }

        if !self.segment_storage
            || !matches!(step, ForStep::Up(_))
            || self.constant_u16(end).is_some()
        {
            return None;
        }

        if target.size == 2
            && self.expr_size(end) == Some(2)
            && let Some(cache) = self.emit_compatible_word_for_end_sub_cache(end, span)
        {
            return Some(cache);
        }

        match target.size {
            1 if self.expr_size(end) == Some(1) && self.is_index_expr(end) => {
                let cache_label = self.next_label("for:end-cache");
                if !self.emit_load_simple_byte(end, 0) {
                    return None;
                }
                self.emit_sta_absolute_label(cache_label.clone(), span);
                Some(ForEndCache::ByteLabel(cache_label))
            }
            2 if self.expr_size(end) == Some(2)
                && (Self::compare_operand_needs_materialization(end)
                    || self.is_index_expr(end)) =>
            {
                let low = self.next_label("for:end-cache-low");
                let high = self.next_label("for:end-cache-high");
                let temp = StorageSlot::zero_page(runtime_zp::ADDR.address(), 2);
                if !self.emit_expr_to_slot(end, temp) {
                    return None;
                }
                self.emit_lda_slot_byte(temp, 0);
                self.emit_sta_absolute_label(low.clone(), span);
                self.emit_lda_slot_byte(temp, 1);
                self.emit_sta_absolute_label(high.clone(), span);
                Some(ForEndCache::WordLabels { low, high })
            }
            _ => None,
        }
    }

    pub(super) fn modern_for_end_cache_width(
        &self,
        target: &Expr,
        end: &Expr,
        step: Option<&Expr>,
    ) -> Option<u16> {
        if !self.segment_storage
            || !self.profile.enables_modern_optimizations()
            || !matches!(
                step.map_or(Some(ForStep::Up(1)), constant_for_step)?,
                ForStep::Up(_)
            )
            || self.constant_u16(end).is_some()
        {
            return None;
        }
        let ExprKind::Name(name) = &target.kind else {
            return None;
        };
        let target = self.lookup_slot(name)?;
        if target.size == 2
            && self.expr_size(end) == Some(2)
            && self.word_for_end_sub_cache_is_supported(end)
        {
            return Some(2);
        }
        match target.size {
            1 if self.expr_size(end) == Some(1) && self.is_index_expr(end) => Some(1),
            2 if self.expr_size(end) == Some(2)
                && (Self::compare_operand_needs_materialization(end)
                    || self.is_index_expr(end)) =>
            {
                Some(2)
            }
            _ => None,
        }
    }

    fn word_for_end_sub_cache_is_supported(&self, end: &Expr) -> bool {
        let ExprKind::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } = &end.kind
        else {
            return false;
        };
        self.constant_u16(right).is_some()
            && self.expr_size(left) == Some(2)
            && self.is_index_expr(left)
    }

    fn emit_modern_for_end_cache(
        &mut self,
        end: &Expr,
        cache: ModernForEndCache,
        _span: Span,
    ) -> Option<ForEndCache> {
        match cache {
            ModernForEndCache::Byte(slot) => {
                if !self.emit_load_simple_byte(end, 0) {
                    return None;
                }
                self.emit_sta_slot_byte(slot, 0);
                Some(ForEndCache::ByteSlot(slot))
            }
            ModernForEndCache::Word { low, high } => {
                if self.emit_word_for_end_sub_cache_to_slots(end, low, high) {
                    return Some(ForEndCache::WordSlots { low, high });
                }
                let temp = StorageSlot::zero_page(runtime_zp::ADDR.address(), 2);
                if !self.emit_expr_to_slot(end, temp) {
                    return None;
                }
                self.emit_lda_slot_byte(temp, 0);
                self.emit_sta_slot_byte(low, 0);
                self.emit_lda_slot_byte(temp, 1);
                self.emit_sta_slot_byte(high, 0);
                Some(ForEndCache::WordSlots { low, high })
            }
        }
    }

    fn emit_compatible_word_for_end_sub_cache(
        &mut self,
        end: &Expr,
        span: Span,
    ) -> Option<ForEndCache> {
        let ExprKind::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } = &end.kind
        else {
            return None;
        };
        let constant = self.constant_u16(right)?;
        if self.expr_size(left) != Some(2) || !self.is_index_expr(left) {
            return None;
        }

        let low = self.next_label("for:end-cache-low");
        let high = self.next_label("for:end-cache-high");
        let source = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR)?;
        if source.size != 2 {
            return None;
        }
        let immediate = Immediate::new(constant);
        self.emit_sec();
        self.emit_lda_slot_byte(source, 0);
        self.emit_sbc_immediate(immediate, 0);
        self.emit_sta_absolute_label(low.clone(), span);
        self.emit_lda_slot_byte(source, 1);
        self.emit_sbc_immediate(immediate, 1);
        self.emit_sta_absolute_label(high.clone(), span);
        Some(ForEndCache::WordLabels { low, high })
    }

    fn emit_word_for_end_sub_cache_to_slots(
        &mut self,
        end: &Expr,
        low: StorageSlot,
        high: StorageSlot,
    ) -> bool {
        let ExprKind::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } = &end.kind
        else {
            return false;
        };
        let Some(constant) = self.constant_u16(right) else {
            return false;
        };
        if self.expr_size(left) != Some(2) || !self.is_index_expr(left) {
            return false;
        }

        let Some(source) = self.reusable_lvalue_slot_with_pointer(left, runtime_zp::ARRAY_ADDR)
        else {
            return false;
        };
        if source.size != 2 {
            return false;
        }
        let immediate = Immediate::new(constant);
        self.emit_sec();
        self.emit_lda_slot_byte(source, 0);
        self.emit_sbc_immediate(immediate, 0);
        self.emit_sta_slot_byte(low, 0);
        self.emit_lda_slot_byte(source, 1);
        self.emit_sbc_immediate(immediate, 1);
        self.emit_sta_slot_byte(high, 0);
        true
    }

    fn emit_for_step_slot(&mut self, slot: StorageSlot, step: ForStep) {
        match step {
            ForStep::Up(amount) => self.emit_increment_slot(slot, amount),
            ForStep::Down(amount)
                if self.segment_storage
                    && amount == 1
                    && slot.space == AddressSpace::Absolute
                    && slot.size == 1 =>
            {
                self.emit_clc();
                self.emit_lda_slot_byte(slot, 0);
                self.emit_adc_imm(0xFF);
                self.emit_sta_slot_byte(slot, 0);
            }
            ForStep::Down(amount) => self.emit_decrement_slot(slot, amount),
        }
    }

    pub(super) fn generate_assignment(&mut self, target: &Expr, value: &Expr, span: Span) {
        if self.emit_array_name_assignment(target, value) {
            return;
        }
        if self.emit_routine_target_assignment(target, value, span) {
            return;
        }
        if self.emit_routine_name_to_card_assignment(target, value, span) {
            return;
        }
        if self.emit_compatible_inline_byte_array_dynamic_assignment(target, value) {
            return;
        }
        if self.emit_compatible_inline_byte_array_constant_assignment(target, value) {
            return;
        }
        if self.emit_inline_byte_array_same_index_assignment(target, value) {
            return;
        }
        if self.emit_inline_byte_array_same_index_add_assignment(target, value) {
            return;
        }
        if self.emit_inline_byte_array_call_index_assignment(target, value) {
            return;
        }
        if self.emit_absolute_x_rhs_assignment_preserving_scalar_index(target, value) {
            return;
        }
        if self.emit_same_effective_address_call_assignment(target, value, span) {
            return;
        }
        if self.emit_effective_address_assignment(target, value) {
            return;
        }

        let Some(slot) = self.lvalue_slot(target) else {
            self.diagnostics.push(Diagnostic::new(
                target.span,
                format!(
                    "codegen only supports scalar variable, constant-index array, and pointer dereference assignment targets: {}",
                    target.text
                ),
            ));
            return;
        };
        debug_assert_assignment_width_shape(
            target,
            value,
            slot,
            self.assignment_value_width(value),
            self.record_value_argument_base(value, slot).is_some(),
        );
        if self.emit_record_value_address_to_slot(value, slot) {
            return;
        }
        if self.emit_indirect_call_assignment_preserving_pointer(slot, value) {
            return;
        }
        if self.emit_scalar_plus_inline_byte_array_index_assignment(slot, value) {
            return;
        }
        if self.emit_indirect_self_word_negation_assignment(target, value, slot) {
            return;
        }
        if self.emit_indirect_self_byte_arithmetic_assignment(target, value, slot) {
            return;
        }
        if self.emit_expanded_compound_bitwise_assignment(target, value, slot) {
            return;
        }
        if self.emit_indirect_rhs_assignment_preserving_pointer(target, slot, value) {
            return;
        }
        let target_index = self.absolute_x_target_index_slot(target);
        if self.emit_absolute_x_rhs_assignment_preserving_index(slot, target_index, value) {
            return;
        }
        if !self.emit_expr_to_slot(value, slot) {
            self.diagnostics.push(Diagnostic::new(
                span,
                "codegen only supports constants, scalar variables, simple arithmetic, logic, arrays, and pointer values",
            ));
        }
    }

    pub(super) fn generate_compound_assignment(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
        span: Span,
    ) {
        if self.emit_compatible_compound_peephole(target, op, value) {
            return;
        }
        if self.emit_indirect_byte_compound_lvalue_direct(target, op, value) {
            return;
        }
        if self.emit_compatible_indirect_word_compound_direct(target, op, value) {
            return;
        }

        let combined = Expr {
            kind: ExprKind::Binary {
                op,
                left: Box::new(target.clone()),
                right: Box::new(value.clone()),
            },
            text: String::new(),
            span,
        };
        self.generate_assignment(target, &combined, span);
    }
}
