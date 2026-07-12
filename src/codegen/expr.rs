use super::*;

impl Generator {
    // Extracted from src/codegen.rs: assignment_value_width
    pub(super) fn assignment_value_width(&self, expr: &Expr) -> Option<u16> {
        match &expr.kind {
            ExprKind::Cast { ty, .. } => cast_type_size(ty),
            ExprKind::Name(name) => {
                let slot = self.lookup_slot(name)?;
                if slot.pointee_size.is_some() || slot.array.is_some() {
                    Some(2)
                } else {
                    Some(slot.size)
                }
            }
            ExprKind::String(_) => Some(2),
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                ..
            } => Some(2),
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::Add | BinaryOp::Sub)
                    && (self.assignment_value_width(left) == Some(2)
                        || self.assignment_value_width(right) == Some(2)) =>
            {
                Some(2)
            }
            _ => self.expr_size(expr),
        }
    }

    // Extracted from src/codegen.rs: peek_lvalue_slot
    pub(super) fn peek_lvalue_slot(&self, expr: &Expr) -> Option<StorageSlot> {
        match &expr.kind {
            ExprKind::Name(name) => self.lookup_slot(name).filter(|slot| slot.array.is_none()),
            ExprKind::Index { base, index } => self.peek_array_element_slot(base, index),
            ExprKind::Call { callee, args } if args.len() == 1 => {
                self.peek_array_element_slot(callee, &args[0])
            }
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let base_slot = self.lookup_slot(name)?;
                if base_slot.pointee_size.is_some() || base_slot.space != AddressSpace::Absolute {
                    return None;
                }
                let field = self.record_layouts.field(base_slot.record?, field)?;
                Some(
                    StorageSlot::absolute(base_slot.address.wrapping_add(field.offset), field.size)
                        .signed(field.signed),
                )
            }
            _ => None,
        }
    }

    // Extracted from src/codegen.rs: peek_array_element_slot
    pub(super) fn peek_array_element_slot(&self, base: &Expr, index: &Expr) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if slot.array != Some(ArrayStorage::Inline) || slot.pointee_size.is_some() {
            return None;
        }
        let index = self.constant_u16(index)?;
        Some(StorageSlot {
            array: None,
            ..slot.offset_bytes(index.saturating_mul(slot.size))
        })
    }

    // Extracted from src/codegen.rs: array_call_slot_size
    pub(super) fn array_call_slot_size(&self, callee: &Expr, args: &[Expr]) -> Option<u16> {
        if args.len() != 1 {
            return None;
        }
        let ExprKind::Name(name) = &callee.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if slot.array.is_some() {
            Some(slot.size)
        } else {
            slot.pointee_size
        }
    }

    // Extracted from src/codegen.rs: array_call_signed
    pub(super) fn array_call_signed(&self, callee: &Expr, args: &[Expr]) -> Option<bool> {
        if args.len() != 1 {
            return None;
        }
        let ExprKind::Name(name) = &callee.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        (slot.array.is_some() || slot.pointee_size.is_some()).then_some(slot.signed)
    }

    // Extracted from src/codegen.rs: array_argument_base
    pub(super) fn array_argument_base(&self, expr: &Expr) -> Option<Absolute> {
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        match slot.array? {
            ArrayStorage::Inline => Some(Absolute::new(slot.address)),
            ArrayStorage::Pointer | ArrayStorage::Descriptor => None,
        }
    }

    // Extracted from src/codegen.rs: record_value_argument_base
    pub(super) fn record_value_argument_base(
        &self,
        expr: &Expr,
        target: StorageSlot,
    ) -> Option<Absolute> {
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        let source = self.lookup_slot(name)?;
        (source.record.is_some()
            && source.array.is_none()
            && source.pointee_size.is_none()
            && source.record == target.record
            && target.pointee_size.is_some())
        .then_some(Absolute::new(source.address))
    }

    // Extracted from src/codegen.rs: direct_scalar_slot
    pub(super) fn direct_scalar_slot(&self, expr: &Expr) -> Option<StorageSlot> {
        match &expr.kind {
            ExprKind::Name(name) => {
                let slot = self.lookup_slot(name)?;
                (matches!(slot.space, AddressSpace::Absolute | AddressSpace::ZeroPage)
                    && slot.array.is_none())
                .then_some(slot)
            }
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let base_slot = self.lookup_slot(name)?;
                if base_slot.pointee_size.is_some() || base_slot.space != AddressSpace::Absolute {
                    return None;
                }
                let field = self.record_layouts.field(base_slot.record?, field)?;
                Some(
                    StorageSlot::absolute(base_slot.address.wrapping_add(field.offset), field.size)
                        .signed(field.signed),
                )
            }
            _ => None,
        }
    }

    // Extracted from src/codegen.rs: call_return_slot
    pub(super) fn call_return_slot(&self, callee: &Expr) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &callee.kind else {
            return None;
        };
        if let Some(pointer) = self.callable_pointer_info(name) {
            return pointer.return_slot;
        }
        self.routines.get(&normalize_name(name)).and_then(|info| {
            if let Some(slot) = info.return_slot {
                debug_assert_call_return_slot_shape(name, slot);
                Some(slot)
            } else {
                None
            }
        })
    }

    // Extracted from src/codegen.rs: call_routine_info
    pub(super) fn call_routine_info(&self, callee: &Expr) -> Option<RoutineInfo> {
        let ExprKind::Name(name) = &callee.kind else {
            return None;
        };
        let info = self.routines.get(&normalize_name(name))?.clone();
        if let Some(slot) = info.return_slot {
            debug_assert_call_return_slot_shape(name, slot);
        }
        Some(info)
    }

    pub(super) fn callable_pointer_info(&self, name: &str) -> Option<CallablePointerInfo> {
        let normalized = normalize_name(name);
        self.local_callable_pointers
            .get(&normalized)
            .or_else(|| self.callable_pointers.get(&normalized))
            .cloned()
    }

    // Extracted from src/codegen.rs: expr_size
    pub(super) fn expr_size(&self, expr: &Expr) -> Option<u16> {
        if let Some(value) = self.constant_u16(expr) {
            return Some(if value > 0xFF { 2 } else { 1 });
        }
        match &expr.kind {
            ExprKind::Name(name) => self.lookup_slot(name).map(|slot| slot.size),
            ExprKind::Index { base, .. } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                self.lookup_slot(name).map(|slot| slot.size)
            }
            ExprKind::Field { base, field } => self
                .record_field_metadata(base, field)
                .map(|field| field.size),
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                ..
            } => Some(2),
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self.pointer_expr_pointee_size(expr),
            ExprKind::Call { callee, args } => {
                if let Some(slot) = self.array_call_slot_size(callee, args) {
                    Some(slot)
                } else {
                    self.call_return_slot(callee).map(|slot| slot.size)
                }
            }
            ExprKind::Cast { ty, .. } => cast_type_size(ty),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Lsh | BinaryOp::Rsh => self.expr_size(left),
                _ => Some(self.expr_size(left)?.max(self.expr_size(right)?)),
            },
            _ => None,
        }
    }

    // Extracted from src/codegen.rs: expr_signed
    pub(super) fn expr_signed(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => self
                .lookup_slot(name)
                .is_some_and(|slot| slot.pointee_size.is_none() && slot.signed),
            ExprKind::Index { base, .. } => {
                let ExprKind::Name(name) = &base.kind else {
                    return false;
                };
                self.lookup_slot(name).is_some_and(|slot| slot.signed)
            }
            ExprKind::Field { base, field } => self
                .record_field_metadata(base, field)
                .is_some_and(|field| field.signed),
            ExprKind::Call { callee, args } => {
                if let Some(signed) = self.array_call_signed(callee, args) {
                    signed
                } else {
                    self.call_return_slot(callee)
                        .is_some_and(|slot| slot.signed)
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self.pointer_expr_pointee_signed(expr),
            ExprKind::Binary { left, right, .. } => {
                self.expr_signed(left) || self.expr_signed(right)
            }
            ExprKind::Cast { ty, expr } => match ty.base {
                TypeBase::Fund(FundType::Int) => true,
                TypeBase::Fund(FundType::Byte | FundType::Card | FundType::Char)
                | TypeBase::Named(_)
                | TypeBase::Callable(_) => self.expr_signed(expr),
            },
            _ => false,
        }
    }

    // Extracted from src/codegen.rs: pointer_expr_pointee_size
    pub(super) fn pointer_expr_pointee_size(&self, expr: &Expr) -> Option<u16> {
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        self.lookup_slot(name)?.pointee_size
    }

    // Extracted from src/codegen.rs: pointer_expr_pointee_signed
    pub(super) fn pointer_expr_pointee_signed(&self, expr: &Expr) -> bool {
        let ExprKind::Name(name) = &expr.kind else {
            return false;
        };
        self.lookup_slot(name)
            .is_some_and(|slot| slot.pointee_size.is_some() && slot.signed)
    }

    // Extracted from src/codegen.rs: record_field_metadata
    pub(super) fn record_field_metadata(&self, base: &Expr, field: &str) -> Option<RecordField> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        self.record_layouts.field(slot.record?, field)
    }

    // Extracted from src/codegen.rs: lvalue_slot
    pub(super) fn lvalue_slot(&mut self, expr: &Expr) -> Option<StorageSlot> {
        let slot = match &expr.kind {
            ExprKind::Name(name) => self.lookup_slot(name).filter(|slot| slot.array.is_none()),
            ExprKind::Index { base, index } => self.index_slot(base, index),
            ExprKind::Field { base, field } => self.record_field_slot(base, field),
            ExprKind::Call { callee, args } => self.array_call_slot(callee, args),
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self.pointer_deref_slot(expr),
            _ => None,
        }?;
        debug_assert_lvalue_slot_shape(expr, slot);
        Some(slot)
    }

    // Extracted from src/codegen.rs: record_field_slot
    pub(super) fn record_field_slot(&mut self, base: &Expr, field: &str) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        let record = slot.record?;
        let field = self.record_layouts.field(record, field)?;
        if slot.pointee_size.is_some() {
            let addr = if self.segment_storage {
                runtime_zp::ARRAY_ADDR
            } else {
                runtime_zp::ADDR
            };
            if self.profile.enables_modern_optimizations() && record_field_fits_indirect_y(field) {
                if !self.emit_pointer_slot_to_addr(slot, addr) {
                    return None;
                }
                let field_slot = StorageSlot::indirect_indexed_y(addr, field.size)
                    .offset_bytes(field.offset)
                    .signed(field.signed);
                debug_assert_prepared_indirect_slot(field_slot, addr, "record field");
                self.processor.mark_prepared_pointer(
                    addr,
                    PreparedPointerFact {
                        key: format!("record-base:{}", normalize_name(name)),
                        deps: vec![slot_dependency(slot)],
                    },
                );
                return Some(field_slot);
            }
            if self.segment_storage && field.offset > 0 {
                if !self.emit_pointer_slot_plus_offset_to_addr(slot, field.offset, addr) {
                    return None;
                }
            } else {
                if !self.emit_pointer_slot_to_addr(slot, addr) {
                    return None;
                }
                self.emit_add_constant_to_addr(addr, field.offset);
            }
            let field_slot = StorageSlot::indirect_indexed_y(addr, field.size).signed(field.signed);
            debug_assert_prepared_indirect_slot(field_slot, addr, "record field");
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

    // Extracted from src/codegen.rs: index_slot
    pub(super) fn index_slot(&mut self, base: &Expr, index: &Expr) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if slot.pointee_size.is_some() {
            return self.pointer_index_slot(slot, index);
        }
        slot.array?;
        if let Some(index) = self.constant_u16(index) {
            return self.constant_index_slot(slot, index);
        }
        if self.segment_storage && slot.array == Some(ArrayStorage::Inline) && slot.size == 1 {
            if let Some(index_slot) = self.direct_scalar_slot(index)
                && index_slot.size == 1
            {
                self.emit_ldx_slot_byte(index_slot, 0);
                return Some(StorageSlot::absolute_x(slot.address, slot.size).signed(slot.signed));
            }
        }
        let pointer = self.emit_dynamic_array_address(slot, index)?;
        let indexed = StorageSlot::indirect_indexed_y(pointer, slot.size).signed(slot.signed);
        debug_assert_prepared_indirect_slot(indexed, pointer, "dynamic array index");
        Some(indexed)
    }

    // Extracted from src/codegen.rs: constant_index_slot
    pub(super) fn constant_index_slot(
        &mut self,
        array: StorageSlot,
        index: u16,
    ) -> Option<StorageSlot> {
        let offset = index.saturating_mul(array.size);
        match array.array? {
            ArrayStorage::Inline => Some(StorageSlot {
                array: None,
                ..array.offset_bytes(offset)
            }),
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                if self.segment_storage && offset > 0 {
                    self.emit_array_base_plus_constant_to_addr(array, offset)?;
                } else {
                    self.emit_array_base_to_addr(array)?;
                    self.emit_add_constant_to_array_addr(offset);
                }
                let indexed = StorageSlot::indirect_indexed_y(runtime_zp::ARRAY_ADDR, array.size)
                    .signed(array.signed);
                debug_assert_prepared_indirect_slot(
                    indexed,
                    runtime_zp::ARRAY_ADDR,
                    "constant array index",
                );
                Some(indexed)
            }
        }
    }

    // Extracted from src/codegen.rs: array_call_slot
    pub(super) fn array_call_slot(&mut self, callee: &Expr, args: &[Expr]) -> Option<StorageSlot> {
        if args.len() != 1 {
            return None;
        }
        self.index_slot(callee, &args[0])
    }

    // Extracted from src/codegen.rs: pointer_deref_slot
    pub(super) fn pointer_deref_slot(&mut self, expr: &Expr) -> Option<StorageSlot> {
        let addr = if self.segment_storage {
            runtime_zp::ARRAY_ADDR
        } else {
            runtime_zp::ADDR
        };
        self.pointer_deref_slot_with_addr(expr, addr)
    }

    // Extracted from src/codegen.rs: pointer_deref_slot_with_addr
    pub(super) fn pointer_deref_slot_with_addr(
        &mut self,
        expr: &Expr,
        addr: ZeroPage,
    ) -> Option<StorageSlot> {
        debug_assert_scratch_indirect_pointer(addr, "pointer dereference");
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        let fact_key = normalize_name(name);
        let pointer = self.lookup_slot(name)?;
        let slot = if pointer.space == AddressSpace::ZeroPage && pointer.address < 0xFF {
            pointer_pointee_slot(pointer, ZeroPage::new(pointer.address as u8))?
        } else {
            if !self.emit_pointer_slot_to_addr(pointer, addr) {
                return None;
            }
            pointer_pointee_slot(pointer, addr)?
        };
        if self.profile.enables_modern_optimizations()
            && slot.space == AddressSpace::IndirectIndexedY
        {
            self.processor.mark_prepared_pointer(
                slot.zero_page_byte(0),
                PreparedPointerFact {
                    key: format!("deref:{fact_key}"),
                    deps: vec![slot_dependency(pointer)],
                },
            );
        }
        Some(slot)
    }

    // Extracted from src/codegen.rs: pointer_index_slot
    pub(super) fn pointer_index_slot(
        &mut self,
        pointer: StorageSlot,
        index: &Expr,
    ) -> Option<StorageSlot> {
        let addr = if self.segment_storage {
            runtime_zp::ARRAY_ADDR
        } else {
            runtime_zp::ADDR
        };
        self.pointer_index_slot_with_addr(pointer, index, addr)
    }

    // Extracted from src/codegen.rs: pointer_index_slot_with_addr
    pub(super) fn pointer_index_slot_with_addr(
        &mut self,
        pointer: StorageSlot,
        index: &Expr,
        addr: ZeroPage,
    ) -> Option<StorageSlot> {
        debug_assert_scratch_indirect_pointer(addr, "pointer index");
        let size = pointer.pointee_size?;
        if let Some(index) = self.constant_u16(index) {
            let offset = index.saturating_mul(size);
            if offset > 0 {
                if !self.emit_pointer_slot_plus_offset_to_addr(pointer, offset, addr) {
                    return None;
                }
            } else if !self.emit_pointer_slot_to_addr(pointer, addr) {
                return None;
            }
            return pointer_pointee_slot(pointer, addr);
        }

        if self.emit_pointer_plus_scaled_byte_index_to_addr(pointer, index, addr) {
            return pointer_pointee_slot(pointer, addr);
        }

        if !self.emit_pointer_slot_to_addr(pointer, addr) {
            return None;
        }
        let temp = if addr == runtime_zp::ADDR {
            runtime_zp::ARRAY_ADDR
        } else {
            runtime_zp::ADDR
        };
        if !self.emit_index_expr_to_temp(index, temp) {
            return None;
        }
        self.emit_lda_zero_page_value_only(temp);
        if size == 2 {
            self.emit_asl_a();
        } else if size != 1 {
            return None;
        }
        self.emit_clc();
        self.emit_adc_zero_page(addr);
        self.emit_sta_zero_page(addr);
        self.emit_lda_zero_page_value_only(temp.offset(1));
        if size == 2 {
            self.emit_rol_a();
        }
        self.emit_adc_zero_page(addr.offset(1));
        self.emit_sta_zero_page(addr.offset(1));
        pointer_pointee_slot(pointer, addr)
    }

    // Extracted from src/codegen.rs: address_of_lvalue
    pub(super) fn address_of_lvalue(&mut self, expr: &Expr) -> Option<Absolute> {
        if let ExprKind::Name(name) = &expr.kind
            && !self.local_symbols.contains_key(&normalize_name(name))
            && let Some(address) = self
                .layout
                .absolute_array_value_addresses
                .get(&normalize_name(name))
                .copied()
        {
            return Some(Absolute::new(address));
        }
        let slot = match &expr.kind {
            ExprKind::Name(name) => self.lookup_slot(name)?,
            ExprKind::Call { callee, args } => self.array_call_slot(callee, args)?,
            _ => self.lvalue_slot(expr)?,
        };
        match slot.space {
            AddressSpace::Absolute | AddressSpace::ZeroPage => Some(Absolute::new(slot.address)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => None,
        }
    }

    // Extracted from src/codegen.rs: pointer_deref_slot_with_pointer_expr
    pub(super) fn pointer_deref_slot_with_pointer_expr(
        &mut self,
        expr: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let ExprKind::Unary {
            op: UnaryOp::Deref,
            expr,
        } = &expr.kind
        else {
            return None;
        };
        self.pointer_deref_slot_with_addr(expr, pointer)
    }
}

fn cast_type_size(ty: &TypeRef) -> Option<u16> {
    if ty.pointer {
        return Some(2);
    }
    match ty.base {
        TypeBase::Fund(FundType::Byte | FundType::Char) => Some(1),
        TypeBase::Fund(FundType::Card | FundType::Int) => Some(2),
        TypeBase::Callable(_) => Some(2),
        TypeBase::Named(_) => None,
    }
}
