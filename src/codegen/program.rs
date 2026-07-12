use super::*;

impl Generator {
    pub(super) fn generate_program(&mut self, program: &Program) {
        let _modern_profile = self.profile.enables_modern_optimizations();
        self.apply_runtime_helper_sets(program);
        if self.segment_storage {
            self.generate_compatible_program_source_order(program);
            return;
        }
        for module in &program.modules {
            for item in &module.items {
                if let Item::Routine(routine) = item {
                    self.generate_routine(routine);
                }
            }
        }
    }

    pub(super) fn generate_compatible_program_source_order(&mut self, program: &Program) {
        let last_routine_position = program
            .modules
            .iter()
            .enumerate()
            .flat_map(|(module_index, module)| {
                module
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(move |(item_index, item)| match item {
                        Item::Routine(routine)
                            if routine_absolute_system_address(routine).is_none() =>
                        {
                            Some((module_index, item_index))
                        }
                        _ => None,
                    })
            })
            .last();

        for (module_index, module) in program.modules.iter().enumerate() {
            for (item_index, item) in module.items.iter().enumerate() {
                match item {
                    Item::Declaration(Decl::Var(decl)) => self.emit_compatible_global_decl(decl),
                    Item::Routine(routine)
                        if self.sync_compatible_cursor_to_emitter(routine.span) =>
                    {
                        self.suppress_implicit_rts_once = last_routine_position
                            == Some((module_index, item_index))
                            && !routine_body_ends_explicitly(routine);
                        self.generate_routine(routine);
                        self.compatible_cursor = Some(self.current_absolute_address());
                    }
                    Item::Set(set) => self.apply_compatible_code_pointer_set(set),
                    _ => {}
                }
            }
        }
        self.sync_compatible_cursor_to_emitter(Span::new(0, 0));
        if !(self.profile.enables_modern_optimizations() && self.last_routine_ended_with_rts) {
            self.emitter.emit_rts();
        } else {
            self.record_modern_optimization(
                CodegenOptimizationKind::FinalRtsRemoved,
                1,
                None,
                "removed duplicate final program RTS",
            );
        }
        self.emit_array_backing_storage();
    }

    pub(super) fn emit_compatible_global_decl(&mut self, decl: &VarDecl) {
        if self.emit_pre_output_pointer_decl(decl) {
            return;
        }

        let address = self
            .compatible_cursor
            .unwrap_or(self.current_absolute_address());
        self.layout.next_address = address;
        let initializer_start = self.layout.initializers.len();
        self.layout
            .add_var_decl(decl, true, &self.record_layouts, &self.numeric_defines);
        let new_initializers = self.layout.initializers[initializer_start..].to_vec();
        if self.compatible_address_is_in_output(address)
            && self.sync_compatible_cursor_to_emitter(decl.span)
        {
            self.emit_storage_initializers_with_source(
                &new_initializers,
                Some(var_decl_source_name(decl)),
                decl.span,
            );
        }
        self.compatible_cursor = Some(self.layout.next_address);
    }

    pub(super) fn emit_pre_output_pointer_decl(&mut self, decl: &VarDecl) -> bool {
        let Some(cursor) = self.compatible_cursor else {
            return false;
        };
        if cursor >= self.emitter.origin || cursor >= 0x0100 || decl_is_array_like(decl) {
            return false;
        }
        let Some(pointee_size) = pointee_size_with_records(&decl.ty, &self.record_layouts) else {
            return false;
        };
        if decl.entries.iter().any(|entry| entry.initializer.is_some()) {
            return false;
        }

        let mut low_cursor = cursor;
        for entry in &decl.entries {
            let address = low_cursor;
            low_cursor = low_cursor.wrapping_add(2);
            if pointee_size > 1 {
                self.deferred_output_cursor = self.deferred_output_cursor.wrapping_add(2);
            }
            self.layout.symbols.insert(
                normalize_name(&entry.name),
                StorageSlot::zero_page_pointer(address as u8, pointee_size)
                    .signed(slot_signed_for_type(&decl.ty)),
            );
        }
        self.compatible_cursor = Some(low_cursor);
        true
    }

    pub(super) fn apply_compatible_code_pointer_set(&mut self, set: &SetDirective) {
        if self.apply_compatible_symbol_memory_set(set) {
            return;
        }
        let Some(address) = self.constant_u16(&set.address) else {
            return;
        };
        let Some(value) = self.constant_u16(&set.value) else {
            return;
        };
        match address {
            0x000E | 0x0491 => {
                self.compatible_cursor = Some(if value >= self.emitter.origin {
                    value.max(self.deferred_output_cursor)
                } else {
                    value
                })
            }
            0x000F | 0x0492 => {
                let current = self
                    .compatible_cursor
                    .unwrap_or(self.current_absolute_address());
                self.compatible_cursor = Some((current & 0x00FF) | ((value & 0x00FF) << 8));
            }
            _ => {}
        }
    }

    pub(super) fn apply_compatible_symbol_memory_set(&mut self, set: &SetDirective) -> bool {
        let ExprKind::Name(name) = &set.address.kind else {
            return false;
        };
        let Some(slot) = self.lookup_slot(name) else {
            return false;
        };
        let Some(value) = self.compatible_set_value(&set.value) else {
            return false;
        };
        let width = compatible_set_storage_width(slot);
        if width == 0 {
            return false;
        }
        self.patch_compatible_absolute_bytes(slot.address, value, width)
    }

    pub(super) fn compatible_set_value(&self, expr: &Expr) -> Option<u16> {
        match expr.kind {
            ExprKind::CurrentLocation => Some(self.current_compatible_high_water()),
            _ => self.constant_u16(expr),
        }
    }

    pub(super) fn current_compatible_location(&self) -> u16 {
        self.compatible_cursor
            .unwrap_or_else(|| self.current_absolute_address())
    }

    fn current_compatible_high_water(&self) -> u16 {
        let current = self.current_compatible_location();
        if self.layout.array_backings.is_empty() {
            return current;
        }

        let final_rts_len =
            if self.profile.enables_modern_optimizations() && self.last_routine_ended_with_rts {
                0
            } else {
                1
            };
        self.layout
            .array_backings
            .iter()
            .fold(current.wrapping_add(final_rts_len), |address, backing| {
                address.wrapping_add(backing.size)
            })
    }

    pub(super) fn patch_compatible_absolute_bytes(
        &mut self,
        address: u16,
        value: u16,
        width: u16,
    ) -> bool {
        if address < self.emitter.origin {
            return false;
        }
        let offset = usize::from(address.wrapping_sub(self.emitter.origin));
        let width = usize::from(width.min(2));
        if offset
            .checked_add(width)
            .is_none_or(|end| end > self.emitter.bytes.len())
        {
            return false;
        }
        let value = Immediate::new(value);
        self.emitter.bytes[offset] = value.low();
        if width > 1 {
            self.emitter.bytes[offset + 1] = value.high();
        }
        true
    }

    pub(super) fn compatible_address_is_in_output(&self, address: u16) -> bool {
        address >= self.emitter.origin
    }

    pub(super) fn sync_compatible_cursor_to_emitter(&mut self, span: Span) -> bool {
        let Some(target) = self.compatible_cursor else {
            return true;
        };
        if !self.compatible_address_is_in_output(target) {
            return false;
        }
        let current = self.current_absolute_address();
        if target < current {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!(
                    "compatible code pointer ${target:04X} is before current output ${current:04X}"
                ),
            ));
            return false;
        }
        self.emitter.emit_zeroes(target.wrapping_sub(current));
        true
    }

    pub(super) fn apply_runtime_helper_sets(&mut self, program: &Program) {
        for module in &program.modules {
            for item in &module.items {
                let Item::Set(set) = item else {
                    continue;
                };
                let Some(address) = self.constant_u16(&set.address) else {
                    continue;
                };
                let Some(value) = self.resolve_set_value(&set.value) else {
                    continue;
                };
                self.runtime_helpers.apply_set(address, value);
            }
        }
    }

    pub(super) fn resolve_set_value(&self, expr: &Expr) -> Option<RuntimeHelperTarget> {
        if let Some(value) = self.constant_u16(expr) {
            return Some(RuntimeHelperTarget::Absolute(Absolute::new(value)));
        }

        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        let routine = self.routines.get(&normalize_name(name))?;
        Some(match routine.system_address {
            Some(address) => RuntimeHelperTarget::Absolute(Absolute::new(address)),
            None => RuntimeHelperTarget::Label(routine.label.clone()),
        })
    }
}
