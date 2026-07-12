use super::*;

impl Generator {
    pub(super) fn reusable_lvalue_slot(&mut self, expr: &Expr) -> Option<StorageSlot> {
        if self.profile.enables_modern_optimizations()
            && let Some(slot) = self.reusable_prepared_lvalue_slot(expr)
        {
            return Some(slot);
        }
        match &expr.kind {
            ExprKind::Name(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => self.lvalue_slot(expr),
            ExprKind::Call { callee, args }
                if self.array_call_slot_size(callee, args).is_some() =>
            {
                self.array_call_slot(callee, args)
            }
            _ => None,
        }
    }

    pub(super) fn reusable_lvalue_slot_with_pointer(
        &mut self,
        expr: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        debug_assert_scratch_indirect_pointer(pointer, "reusable lvalue");
        let prepared_fact = self.prepared_pointer_fact(expr);
        if self.profile.enables_modern_optimizations()
            && let Some(fact) = prepared_fact.as_ref()
            && self.processor.prepared_pointer_matches(pointer, &fact.key)
            && let Some(slot) = self.prepared_lvalue_slot(expr, pointer)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::PointerReloadRemoved,
                4,
                Some(expr.span),
                format!(
                    "reused prepared pointer ${:02X}/${:02X} for {}",
                    pointer.address(),
                    pointer.offset(1).address(),
                    fact.key
                ),
            );
            return Some(slot);
        }

        let slot = match &expr.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self.pointer_deref_slot_with_addr(expr, pointer),
            ExprKind::Field { base, field } => {
                self.record_field_slot_with_pointer(base, field, pointer)
            }
            ExprKind::Index { base, index } => self.index_slot_with_pointer(base, index, pointer),
            ExprKind::Call { callee, args } if args.len() == 1 => {
                self.index_slot_with_pointer(callee, &args[0], pointer)
            }
            _ => None,
        };
        if let (Some(fact), Some(slot)) = (prepared_fact, slot)
            && slot.space == AddressSpace::IndirectIndexedY
            && slot.zero_page_byte(0) == pointer
        {
            self.processor.mark_prepared_pointer(pointer, fact);
        }
        slot
    }

    pub(super) fn reusable_lvalue_slot_with_pointer_or_direct(
        &mut self,
        expr: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        if self.prepared_pointer_fact(expr).is_some() {
            return self.reusable_lvalue_slot_with_pointer(expr, pointer);
        }
        let slot = self.reusable_lvalue_slot(expr)?;
        matches!(slot.space, AddressSpace::Absolute | AddressSpace::ZeroPage).then_some(slot)
    }

    pub(super) fn reusable_prepared_lvalue_slot(&mut self, expr: &Expr) -> Option<StorageSlot> {
        if !self.profile.enables_modern_optimizations() {
            return None;
        }
        let fact = self.prepared_pointer_fact(expr)?;
        for pointer in TRACKED_POINTERS.map(ZeroPage::new) {
            if self.processor.prepared_pointer_matches(pointer, &fact.key)
                && let Some(slot) = self.prepared_lvalue_slot(expr, pointer)
            {
                self.record_modern_optimization(
                    CodegenOptimizationKind::PointerReloadRemoved,
                    4,
                    Some(expr.span),
                    format!(
                        "reused prepared pointer ${:02X}/${:02X} for simple load {}",
                        pointer.address(),
                        pointer.offset(1).address(),
                        fact.key
                    ),
                );
                return Some(slot);
            }
        }
        None
    }

    pub(super) fn lvalue_can_be_prepared_or_direct(&self, expr: &Expr) -> bool {
        self.prepared_pointer_fact(expr).is_some()
            || self.direct_scalar_slot(expr).is_some_and(|slot| {
                matches!(slot.space, AddressSpace::Absolute | AddressSpace::ZeroPage)
            })
    }

    fn prepared_lvalue_slot(&self, expr: &Expr, pointer: ZeroPage) -> Option<StorageSlot> {
        match &expr.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let ExprKind::Name(name) = &expr.kind else {
                    return None;
                };
                let source = self.lookup_slot(name)?;
                pointer_pointee_slot(source, pointer)
            }
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let slot = self.lookup_slot(name)?;
                slot.pointee_size?;
                let record = slot.record?;
                let field = self.record_layouts.field(record, field)?;
                if !record_field_fits_indirect_y(field) {
                    return None;
                }
                Some(
                    StorageSlot::indirect_indexed_y(pointer, field.size)
                        .offset_bytes(field.offset)
                        .signed(field.signed),
                )
            }
            ExprKind::Index { base, index } => self.prepared_index_slot(base, index, pointer),
            ExprKind::Call { callee, args } if args.len() == 1 => {
                self.prepared_index_slot(callee, &args[0], pointer)
            }
            _ => None,
        }
    }

    fn prepared_index_slot(
        &self,
        base: &Expr,
        _index: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if let Some(size) = slot.pointee_size {
            return Some(StorageSlot::indirect_indexed_y(pointer, size).signed(slot.signed));
        }
        if !matches!(
            slot.array?,
            ArrayStorage::Pointer | ArrayStorage::Descriptor
        ) {
            return None;
        }
        Some(StorageSlot::indirect_indexed_y(pointer, slot.size).signed(slot.signed))
    }

    pub(super) fn prepared_pointer_fact(&self, expr: &Expr) -> Option<PreparedPointerFact> {
        match &expr.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let ExprKind::Name(name) = &expr.kind else {
                    return None;
                };
                let slot = self.lookup_slot(name)?;
                Some(PreparedPointerFact {
                    key: format!("deref:{}", normalize_name(name)),
                    deps: vec![slot_dependency(slot)],
                })
            }
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let slot = self.lookup_slot(name)?;
                slot.pointee_size?;
                let record = slot.record?;
                let field_layout = self.record_layouts.field(record, field)?;
                let key = if self.profile.enables_modern_optimizations()
                    && record_field_fits_indirect_y(field_layout)
                {
                    format!("record-base:{}", normalize_name(name))
                } else {
                    format!(
                        "field:{}:{}:{}",
                        normalize_name(name),
                        normalize_name(field),
                        field_layout.offset
                    )
                };
                Some(PreparedPointerFact {
                    key,
                    deps: vec![slot_dependency(slot)],
                })
            }
            ExprKind::Index { base, index } => self.prepared_index_fact(base, index),
            ExprKind::Call { callee, args } if args.len() == 1 => {
                self.prepared_index_fact(callee, &args[0])
            }
            _ => None,
        }
    }

    fn prepared_index_fact(&self, base: &Expr, index: &Expr) -> Option<PreparedPointerFact> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if slot.pointee_size.is_none()
            && !matches!(
                slot.array?,
                ArrayStorage::Pointer | ArrayStorage::Descriptor
            )
        {
            return None;
        }
        let index_key = self.prepared_index_key(index)?;
        let mut deps = vec![slot_dependency(slot)];
        self.push_index_dependency(index, &mut deps);
        Some(PreparedPointerFact {
            key: format!(
                "index:{}:{}:{}",
                normalize_name(name),
                slot.pointee_size.unwrap_or(slot.size),
                index_key
            ),
            deps,
        })
    }

    fn prepared_index_key(&self, expr: &Expr) -> Option<String> {
        if let Some(value) = self.constant_u16(expr) {
            return Some(format!("#{value}"));
        }
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        Some(normalize_name(name))
    }

    fn push_index_dependency(&self, expr: &Expr, deps: &mut Vec<PreparedDependency>) {
        let ExprKind::Name(name) = &expr.kind else {
            return;
        };
        if let Some(slot) = self.lookup_slot(name) {
            deps.push(slot_dependency(slot));
        }
    }

    fn record_field_slot_with_pointer(
        &mut self,
        base: &Expr,
        field: &str,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        let record = slot.record?;
        let field = self.record_layouts.field(record, field)?;
        if slot.pointee_size.is_some() {
            if self.profile.enables_modern_optimizations() && record_field_fits_indirect_y(field) {
                if !self.emit_pointer_slot_to_addr(slot, pointer) {
                    return None;
                }
                let field_slot = StorageSlot::indirect_indexed_y(pointer, field.size)
                    .offset_bytes(field.offset)
                    .signed(field.signed);
                debug_assert_prepared_indirect_slot(field_slot, pointer, "record field");
                return Some(field_slot);
            }
            let emitted = if field.offset > 0 {
                self.emit_pointer_slot_plus_offset_to_addr(slot, field.offset, pointer)
            } else {
                self.emit_pointer_slot_to_addr(slot, pointer)
            };
            if !emitted {
                return None;
            }
            let field_slot =
                StorageSlot::indirect_indexed_y(pointer, field.size).signed(field.signed);
            debug_assert_prepared_indirect_slot(field_slot, pointer, "record field");
            return Some(field_slot);
        }
        if slot.space != AddressSpace::Absolute {
            return None;
        }
        Some(
            StorageSlot::absolute(slot.address.wrapping_add(field.offset), field.size)
                .signed(field.signed),
        )
    }
}
