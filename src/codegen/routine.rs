use super::*;

impl Generator {
    pub(super) fn generate_routine(&mut self, routine: &Routine) {
        if routine_absolute_system_address(routine).is_some() {
            return;
        }
        let previous_profile = self.profile;
        let debug_compat_profile =
            self.profile == CodegenProfile::Modern && routine_uses_debug_compat_profile(routine);
        let routine_start = self.current_absolute_address();
        self.last_routine_label = Some(format!("routine:{}", routine.name));
        self.processor.invalidate_after_call();
        self.straight_line_store_y = None;
        self.label_store_y_hints.clear();
        self.label_byte_values.clear();

        let previous_locals = std::mem::take(&mut self.local_symbols);
        let previous_local_callable_pointers = std::mem::take(&mut self.local_callable_pointers);
        let previous_return_slot = self.current_return_slot;
        let previous_machine_defines = self.machine_defines.clone();
        let previous_routine_effects = self.current_routine_effects;
        let previous_routine_has_effect_contract = self.current_routine_has_effect_contract;
        let previous_inferred_routine_facts = self.current_inferred_routine_facts;
        let previous_modern_routine_layout =
            std::mem::take(&mut self.current_modern_routine_layout);
        let previous_preserve_modern_routine_layout = self.preserve_modern_routine_layout;
        self.current_routine_effects = Some(RoutineEffects::known_empty());
        self.current_routine_has_effect_contract = routine_has_effect_contract(routine);
        self.current_inferred_routine_facts = None;
        self.preserve_modern_routine_layout = false;
        let pool_current_location_strings =
            self.current_location_routine_needs_hidden_storage(routine);

        self.local_symbols = if routine_is_current_location(routine) {
            let symbols = self.allocate_routine_storage(routine, false);
            if pool_current_location_strings {
                let body_label = format!("routine:{}:body", routine.name);
                self.emit_routine_entry_trampoline(routine, &body_label);
                self.emit_modern_routine_hidden_storage(routine, &symbols);
                self.bind_codegen_label(body_label, routine.span);
            } else {
                if let Err(diagnostic) = self
                    .emitter
                    .bind_label(format!("routine:{}", routine.name), routine.span)
                {
                    self.diagnostics.push(diagnostic);
                }
                self.record_routine_address(&routine.name);
            }
            self.record_routine_storage_symbols(routine, &symbols);
            symbols
        } else if self.segment_storage {
            if !debug_compat_profile && self.is_modern_abs_return_routine(routine) {
                if let Err(diagnostic) = self
                    .emitter
                    .bind_label(format!("routine:{}", routine.name), routine.span)
                {
                    self.diagnostics.push(diagnostic);
                }
                self.record_routine_address(&routine.name);
                HashMap::new()
            } else {
                let symbols = self.emit_compatible_routine_preamble(routine);
                self.record_routine_storage_symbols(routine, &symbols);
                symbols
            }
        } else {
            if let Err(diagnostic) = self
                .emitter
                .bind_label(format!("routine:{}", routine.name), routine.span)
            {
                self.diagnostics.push(diagnostic);
            }
            self.record_routine_address(&routine.name);
            let symbols = self.allocate_routine_storage(routine, true);
            self.record_routine_storage_symbols(routine, &symbols);
            symbols
        };
        self.local_callable_pointers = collect_routine_callable_pointers(routine);
        self.current_return_slot = self
            .routines
            .get(&normalize_name(&routine.name))
            .and_then(|info| info.return_slot);
        if let Some(info) = self.routines.get(&normalize_name(&routine.name)) {
            self.routine_signatures
                .push(codegen_routine_signature_from_ast(
                    routine,
                    &info.params,
                    info.return_slot,
                ));
        }
        if let Some(return_slot) = self.current_return_slot.filter(|slot| {
            slot.space == AddressSpace::ZeroPage
                && slot.address == u16::from(runtime_zp::ARGS.address())
                && matches!(slot.size, 1 | 2)
        }) {
            self.current_inferred_routine_facts = Some(InferredRoutineFacts {
                returns_a_equals_a0_candidate: true,
                returns_a_equals_a1_candidate: return_slot.size == 2,
                saw_value_return: false,
            });
        }

        let mut emitted_specialized_body =
            !routine_is_current_location(routine) && self.emit_modern_abs_return_routine(routine);

        if !routine_is_current_location(routine) && !emitted_specialized_body {
            self.emit_routine_parameter_prologue(routine);
        }
        if debug_compat_profile {
            self.profile = CodegenProfile::Compat;
            self.preserve_modern_routine_layout = true;
        }
        if !emitted_specialized_body {
            emitted_specialized_body = self.emit_modern_tail_call_routine_body(routine);
        }
        if !emitted_specialized_body {
            self.generate_stmt_list(&routine.body);
        }
        let suppress_implicit_rts = self.suppress_implicit_rts_once;
        self.suppress_implicit_rts_once = false;
        let should_emit_implicit_rts = !(suppress_implicit_rts
            || emitted_specialized_body
            || routine_is_current_location(routine)
            || self.segment_storage && routine.body.is_empty()
            || self.profile.enables_modern_optimizations()
                && stmt_list_ends_with_terminal_flow(&routine.body)
            || routine_body_ends_explicitly(routine));
        if should_emit_implicit_rts {
            self.emit_return_rts(routine.span);
        }
        let routine_start_position = routine_start.wrapping_sub(self.emitter.origin) as usize;
        self.finalize_modern_branch_inversions(routine_start_position);
        self.last_routine_ended_with_rts =
            !routine.body.is_empty() && self.emitter.bytes.last() == Some(&opcode::RTS);
        self.routine_ranges.push(RoutineRange {
            name: routine.name.clone(),
            start: routine_start,
            end: self.current_absolute_address(),
        });
        self.record_source_range(
            CodegenSourceRangeKind::Routine,
            Some(routine.name.clone()),
            routine.span,
            routine_start,
            self.current_absolute_address(),
        );
        let routine_effects = self
            .current_routine_effects
            .take()
            .unwrap_or_else(RoutineEffects::unknown);
        let routine_effects = self.routine_effects_from_annotations(
            routine_effects,
            &routine.annotations,
            &routine.name,
            routine.span,
        );
        let mut routine_facts = routine_facts_from_annotations(&routine.annotations);
        if !debug_compat_profile {
            if self
                .current_inferred_routine_facts
                .is_some_and(|facts| facts.returns_a_equals_a0_candidate && facts.saw_value_return)
                && stmt_list_ends_with_value_return(&routine.body)
            {
                routine_facts.returns_a_equals_a0 = true;
            }
            if self
                .current_inferred_routine_facts
                .is_some_and(|facts| facts.returns_a_equals_a1_candidate && facts.saw_value_return)
                && stmt_list_ends_with_value_return(&routine.body)
            {
                routine_facts.returns_a_equals_a1 = true;
            }
        }
        if let Some(info) = self.routines.get_mut(&normalize_name(&routine.name)) {
            info.facts = routine_facts;
            info.effects = routine_effects;
        }

        self.local_symbols = previous_locals;
        self.local_callable_pointers = previous_local_callable_pointers;
        self.current_return_slot = previous_return_slot;
        self.machine_defines = previous_machine_defines;
        self.current_routine_effects = previous_routine_effects;
        self.current_routine_has_effect_contract = previous_routine_has_effect_contract;
        self.current_inferred_routine_facts = previous_inferred_routine_facts;
        self.current_modern_routine_layout = previous_modern_routine_layout;
        self.preserve_modern_routine_layout = previous_preserve_modern_routine_layout;
        self.profile = previous_profile;
    }

    fn emit_modern_abs_return_routine(&mut self, routine: &Routine) -> bool {
        if !self.is_modern_abs_return_routine(routine) {
            return false;
        }
        let Some(return_slot) = self.current_return_slot else {
            return false;
        };
        if return_slot.space != AddressSpace::ZeroPage
            || return_slot.address != u16::from(runtime_zp::ARGS.address())
            || return_slot.size != 2
            || !return_slot.signed
        {
            return false;
        }

        let done_label = self.next_label("abs:done");
        self.emit_sta_zero_page(runtime_zp::ARGS);
        self.emit_stx_zero_page(runtime_zp::ARGS.offset(1));
        self.emit_txa();
        self.emit_compare_branch_label(
            opcode::BPL_REL,
            CompareBranchFlags::SignedOrder,
            &done_label,
            routine.span,
        );
        self.emit_sec();
        self.emit_lda_imm(0);
        self.emit_sbc_zero_page(runtime_zp::ARGS);
        self.emit_sta_zero_page(runtime_zp::ARGS);
        self.emit_lda_imm(0);
        self.emit_sbc_zero_page(runtime_zp::ARGS.offset(1));
        self.emit_sta_zero_page(runtime_zp::ARGS.offset(1));
        self.bind_codegen_label(done_label, routine.span);
        self.emit_return_rts(routine.span);
        true
    }

    fn is_modern_abs_return_routine(&self, routine: &Routine) -> bool {
        if !self.profile.enables_modern_optimizations() {
            return false;
        }
        let Some(param_name) = single_int_scalar_param_name(routine) else {
            return false;
        };
        routine_body_is_abs_return(&routine.body, param_name)
    }

    fn record_routine_address(&mut self, name: &str) {
        self.routine_addresses.push(RoutineAddress {
            name: name.to_string(),
            address: self.current_absolute_address(),
        });
    }

    fn record_routine_storage_symbols(
        &mut self,
        routine: &Routine,
        symbols: &HashMap<String, StorageSlot>,
    ) {
        let mut parameter_names = std::collections::HashSet::new();
        for param in &routine.params {
            for entry in &param.entries {
                parameter_names.insert(normalize_name(&entry.name));
            }
        }

        self.storage_symbols
            .extend(symbols.iter().map(|(name, slot)| {
                let kind = if parameter_names.contains(name) {
                    CodegenSymbolKind::Parameter
                } else {
                    CodegenSymbolKind::Local
                };
                codegen_storage_symbol(
                    name.clone(),
                    CodegenSymbolScope::Routine(routine.name.clone()),
                    kind,
                    *slot,
                )
            }));
    }

    fn emit_compatible_routine_preamble(
        &mut self,
        routine: &Routine,
    ) -> HashMap<String, StorageSlot> {
        let storage_base = self
            .emitter
            .origin
            .wrapping_add(self.emitter.position() as u16);
        let allocation = allocate_routine_symbols(
            routine,
            storage_base,
            &self.record_layouts,
            !self.profile.enables_modern_optimizations(),
            &self.numeric_defines,
            &self.layout.symbols,
        );
        self.emit_storage_initializers_with_source(
            &allocation.initializers,
            Some(format!("{} storage", routine.name)),
            routine.span,
        );
        self.emit_modern_routine_hidden_storage(routine, &allocation.symbols);
        self.layout
            .machine_symbol_addresses
            .extend(allocation.machine_symbol_addresses.clone());

        let entry_plan = self.routine_entry_plan(routine);
        self.layout
            .array_backings
            .splice(0..0, allocation.array_backings.into_iter().rev());
        if entry_plan.is_direct() {
            self.bind_routine_entry(routine);
            self.record_modern_optimization(
                CodegenOptimizationKind::TrampolineElided,
                3,
                Some(routine.span),
                format!("elided entry trampoline for {}", routine.name),
            );
            return allocation.symbols;
        }

        let body_label = format!("routine:{}:body", routine.name);
        self.emit_routine_entry_trampoline(routine, &body_label);
        self.bind_codegen_label(body_label, routine.span);

        allocation.symbols
    }

    fn bind_routine_entry(&mut self, routine: &Routine) {
        if let Err(diagnostic) = self
            .emitter
            .bind_label(format!("routine:{}", routine.name), routine.span)
        {
            self.diagnostics.push(diagnostic);
        }
        self.record_routine_address(&routine.name);
    }

    fn emit_routine_entry_trampoline(&mut self, routine: &Routine, body_label: &str) {
        self.bind_routine_entry(routine);
        let trampoline_operand = self.emitter.position().saturating_add(1);
        if let Err(diagnostic) = self.emitter.bind_label_at_position(
            routine_trampoline_operand_label(&routine.name, 0),
            trampoline_operand,
            routine.span,
        ) {
            self.diagnostics.push(diagnostic);
        }
        if let Err(diagnostic) = self.emitter.bind_label_at_position(
            routine_trampoline_operand_label(&routine.name, 1),
            trampoline_operand.saturating_add(1),
            routine.span,
        ) {
            self.diagnostics.push(diagnostic);
        }
        self.emit_jmp_label(body_label, routine.span);
    }

    fn emit_modern_routine_hidden_storage(
        &mut self,
        routine: &Routine,
        symbols: &HashMap<String, StorageSlot>,
    ) {
        if !self.profile.enables_modern_optimizations() || !self.segment_storage {
            return;
        }

        let previous_locals = std::mem::replace(&mut self.local_symbols, symbols.clone());
        self.collect_modern_hidden_stmt_list(&routine.body);
        self.local_symbols = previous_locals;
    }

    fn current_location_routine_needs_hidden_storage(&self, routine: &Routine) -> bool {
        routine_is_current_location(routine)
            && self.profile.enables_modern_optimizations()
            && self.segment_storage
            && stmt_list_contains_string_literal(&routine.body)
    }

    fn collect_modern_hidden_stmt_list(&mut self, body: &[Stmt]) {
        for stmt in body {
            self.collect_modern_hidden_stmt(stmt);
        }
    }

    fn collect_modern_hidden_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { target, value, .. } => {
                self.collect_modern_hidden_expr(target);
                self.collect_modern_hidden_expr(value);
            }
            Stmt::CompoundAssign { target, value, .. } => {
                self.collect_modern_hidden_expr(target);
                self.collect_modern_hidden_expr(value);
            }
            Stmt::Define(_) | Stmt::Unsupported { .. } => {}
            Stmt::Return(Some(expr)) => self.collect_modern_hidden_expr(expr),
            Stmt::Return(None) | Stmt::Exit { .. } | Stmt::MachineBlock { .. } => {}
            Stmt::Call { expr, .. } => self.collect_modern_hidden_expr(expr),
            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    self.collect_modern_hidden_expr(&branch.condition);
                    self.collect_modern_hidden_stmt_list(&branch.body);
                }
                self.collect_modern_hidden_stmt_list(else_body);
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.collect_modern_hidden_expr(condition);
                self.collect_modern_hidden_stmt_list(body);
            }
            Stmt::DoUntil {
                condition, body, ..
            } => {
                if let Some(condition) = condition {
                    self.collect_modern_hidden_expr(condition);
                }
                self.collect_modern_hidden_stmt_list(body);
            }
            Stmt::For {
                target,
                start,
                end,
                step,
                body,
                span,
            } => {
                self.collect_modern_for_end_cache(target, end, step.as_ref(), *span);
                self.collect_modern_hidden_expr(target);
                self.collect_modern_hidden_expr(start);
                self.collect_modern_hidden_expr(end);
                if let Some(step) = step {
                    self.collect_modern_hidden_expr(step);
                }
                self.collect_modern_hidden_stmt_list(body);
            }
        }
    }

    fn collect_modern_hidden_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::String(text) => {
                let key = StringLiteralKey::new(expr.span, text);
                if self
                    .current_modern_routine_layout
                    .string_literals
                    .contains_key(&key)
                {
                    return;
                }
                let address = self.emit_modern_hidden_string_literal(text, expr.span);
                self.current_modern_routine_layout
                    .string_literals
                    .insert(key, address);
            }
            ExprKind::Unary { expr, .. } => self.collect_modern_hidden_expr(expr),
            ExprKind::Cast { expr, .. } => self.collect_modern_hidden_expr(expr),
            ExprKind::Binary { left, right, .. } => {
                self.collect_modern_hidden_expr(left);
                self.collect_modern_hidden_expr(right);
            }
            ExprKind::Call { callee, args } => {
                self.collect_modern_hidden_expr(callee);
                for arg in args {
                    self.collect_modern_hidden_expr(arg);
                }
            }
            ExprKind::Field { base: inner, .. } => self.collect_modern_hidden_expr(inner),
            ExprKind::Index { base, index } => {
                self.collect_modern_hidden_expr(base);
                self.collect_modern_hidden_expr(index);
            }
            ExprKind::Number(_)
            | ExprKind::Char(_)
            | ExprKind::Name(_)
            | ExprKind::CurrentLocation
            | ExprKind::Missing
            | ExprKind::Raw => {}
        }
    }

    fn collect_modern_for_end_cache(
        &mut self,
        target: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        span: Span,
    ) {
        let Some(width) = self.modern_for_end_cache_width(target, end, step) else {
            return;
        };
        let key = SpanKey::from(span);
        if self
            .current_modern_routine_layout
            .for_end_caches
            .contains_key(&key)
        {
            return;
        }
        let cache = match width {
            1 => ModernForEndCache::Byte(self.emit_modern_hidden_slot(
                1,
                "modern hidden FOR end cache",
                span,
            )),
            2 => ModernForEndCache::Word {
                low: self.emit_modern_hidden_slot(1, "modern hidden FOR end cache low", span),
                high: self.emit_modern_hidden_slot(1, "modern hidden FOR end cache high", span),
            },
            _ => return,
        };
        self.current_modern_routine_layout
            .for_end_caches
            .insert(key, cache);
    }

    fn emit_modern_hidden_slot(&mut self, size: u16, name: &str, span: Span) -> StorageSlot {
        let start = self.current_absolute_address();
        self.emitter.emit_zeroes(size);
        let end = self.current_absolute_address();
        self.record_source_range(
            CodegenSourceRangeKind::StorageInitializer,
            Some(name.to_string()),
            span,
            start,
            end,
        );
        StorageSlot::absolute(start, size)
    }

    fn emit_modern_hidden_string_literal(&mut self, text: &str, span: Span) -> Absolute {
        let start = self.current_absolute_address();
        for byte in string_literal_storage(text) {
            self.emitter.emit_u8(byte);
        }
        let end = self.current_absolute_address();
        self.record_source_range(
            CodegenSourceRangeKind::StorageInitializer,
            Some("modern hidden string literal".to_string()),
            span,
            start,
            end,
        );
        Absolute::new(start)
    }

    pub(super) fn record_inferred_return_facts(&mut self, slot: StorageSlot) {
        let Some(facts) = &mut self.current_inferred_routine_facts else {
            return;
        };
        facts.saw_value_return = true;
        if slot.space != AddressSpace::ZeroPage
            || slot.address != u16::from(runtime_zp::ARGS.address())
            || !matches!(slot.size, 1 | 2)
        {
            facts.returns_a_equals_a0_candidate = false;
            facts.returns_a_equals_a1_candidate = false;
            return;
        }
        let accumulator = self.processor.a_value_fact();
        if slot.size == 1 {
            let value = ValueFact::SlotByte {
                slot,
                byte_index: 0,
            };
            let memory = self.processor.memory_value(slot, 0);
            if !self.processor.accumulator_matches_load_result(value) && memory != Some(accumulator)
            {
                facts.returns_a_equals_a0_candidate = false;
            }
            facts.returns_a_equals_a1_candidate = false;
            return;
        }

        let high = ValueFact::SlotByte {
            slot,
            byte_index: 1,
        };
        let high_memory = self.processor.memory_value(slot, 1);
        if !self.processor.accumulator_matches_load_result(high) && high_memory != Some(accumulator)
        {
            facts.returns_a_equals_a1_candidate = false;
        }
        let low = ValueFact::SlotByte {
            slot,
            byte_index: 0,
        };
        let low_memory = self.processor.memory_value(slot, 0);
        if !self.processor.accumulator_matches_load_result(low) && low_memory != Some(accumulator) {
            facts.returns_a_equals_a0_candidate = false;
        }
    }

    fn allocate_routine_storage(
        &mut self,
        routine: &Routine,
        include_params: bool,
    ) -> HashMap<String, StorageSlot> {
        // Use globals to resolve local address aliases, but do not expose them
        // as routine locals after allocation.
        let mut symbols = self.layout.symbols.clone();
        let mut routine_names = std::collections::HashSet::new();
        if include_params {
            for param in &routine.params {
                let Some(element_size) = type_size_with_records(&param.ty, &self.record_layouts)
                else {
                    continue;
                };
                let slot_size = if decl_is_array_like(param) {
                    2
                } else if let Some(slot_size) =
                    storage_size_with_records(&param.ty, &self.record_layouts)
                {
                    slot_size
                } else {
                    continue;
                };
                let pointee_size = pointee_size_with_records(&param.ty, &self.record_layouts);
                for entry in &param.entries {
                    let address = self.layout.allocate(slot_size);
                    let slot = if decl_is_array_like(param) {
                        StorageSlot::array(address, element_size, ArrayStorage::Pointer)
                    } else if let Some(pointee_size) = pointee_size {
                        StorageSlot::pointer(address, pointee_size)
                    } else {
                        StorageSlot::absolute(address, slot_size)
                    }
                    .record(record_id_for_type(&param.ty, &self.record_layouts))
                    .signed(slot_signed_for_type(&param.ty));
                    let name = normalize_name(&entry.name);
                    routine_names.insert(name.clone());
                    symbols.insert(name, slot);
                }
            }
        }

        for local in &routine.locals {
            if let Decl::Var(decl) = local {
                routine_names.extend(decl.entries.iter().map(|entry| normalize_name(&entry.name)));
                add_var_decl_to_symbols(
                    &mut symbols,
                    decl,
                    &self.record_layouts,
                    &self.numeric_defines,
                    routine_is_current_location(routine),
                    |size| self.layout.allocate(size),
                );
            }
        }

        symbols
            .into_iter()
            .filter(|(name, _)| routine_names.contains(name))
            .collect()
    }
}

fn stmt_list_contains_string_literal(body: &[Stmt]) -> bool {
    body.iter().any(stmt_contains_string_literal)
}

fn stmt_contains_string_literal(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
            expr_contains_string_literal(target) || expr_contains_string_literal(value)
        }
        Stmt::Return(Some(expr)) | Stmt::Call { expr, .. } => expr_contains_string_literal(expr),
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            branches.iter().any(|branch| {
                expr_contains_string_literal(&branch.condition)
                    || stmt_list_contains_string_literal(&branch.body)
            }) || stmt_list_contains_string_literal(else_body)
        }
        Stmt::While {
            condition, body, ..
        } => expr_contains_string_literal(condition) || stmt_list_contains_string_literal(body),
        Stmt::DoUntil {
            body, condition, ..
        } => {
            condition.as_ref().is_some_and(expr_contains_string_literal)
                || stmt_list_contains_string_literal(body)
        }
        Stmt::For {
            target,
            start,
            end,
            step,
            body,
            ..
        } => {
            expr_contains_string_literal(target)
                || expr_contains_string_literal(start)
                || expr_contains_string_literal(end)
                || step.as_ref().is_some_and(expr_contains_string_literal)
                || stmt_list_contains_string_literal(body)
        }
        Stmt::Define(_)
        | Stmt::Return(None)
        | Stmt::Exit { .. }
        | Stmt::MachineBlock { .. }
        | Stmt::Unsupported { .. } => false,
    }
}

fn expr_contains_string_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::String(_) => true,
        ExprKind::Unary { expr, .. } => expr_contains_string_literal(expr),
        ExprKind::Cast { expr, .. } => expr_contains_string_literal(expr),
        ExprKind::Binary { left, right, .. } => {
            expr_contains_string_literal(left) || expr_contains_string_literal(right)
        }
        ExprKind::Call { callee, args } => {
            expr_contains_string_literal(callee) || args.iter().any(expr_contains_string_literal)
        }
        ExprKind::Field { base, .. } => expr_contains_string_literal(base),
        ExprKind::Index { base, index } => {
            expr_contains_string_literal(base) || expr_contains_string_literal(index)
        }
        ExprKind::Number(_)
        | ExprKind::Char(_)
        | ExprKind::Name(_)
        | ExprKind::CurrentLocation
        | ExprKind::Missing
        | ExprKind::Raw => false,
    }
}
