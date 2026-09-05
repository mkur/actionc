//! Addressable array bases share canonical element facts and address staging.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct IndexElement {
    pub size: u16,
    pub signed: bool,
    pub record: Option<usize>,
    pub is_volatile: bool,
}

impl Generator {
    pub(super) fn emit_captured_compound_assignment(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
        span: Span,
    ) -> bool {
        let Some(slot) = self.lvalue_slot(target) else {
            return false;
        };
        if !matches!(slot.size, 1 | 2) {
            return false;
        }
        let pointer = runtime_zp::ARRAY_ADDR;
        if !self.emit_slot_address(slot, pointer) {
            return false;
        }
        let destination = StorageSlot::indirect_indexed_y(pointer, slot.size)
            .signed(slot.signed)
            .volatile(slot.is_volatile);
        self.emit_lda_zero_page_value_only(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page_value_only(pointer);
        self.emitter.emit_pha();
        // Both the destination address and its old value precede RHS effects.
        for byte in 0..slot.size {
            self.emit_lda_slot_byte(destination, byte);
            self.emitter.emit_pha();
        }
        let right = StorageSlot::zero_page(
            runtime_zp::ARGS.address(),
            self.expr_size(value).unwrap_or(slot.size),
        )
        .signed(self.expr_signed(value));
        if !self.emit_expr_to_slot(value, right) {
            return false;
        }
        let left =
            StorageSlot::zero_page(runtime_zp::VALUE_TEMP.address(), slot.size).signed(slot.signed);
        for byte in (0..slot.size).rev() {
            self.emit_pla();
            self.emit_sta_slot_byte(left, byte);
        }
        let result = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), slot.size)
            .signed(slot.signed);
        // Reuse the ordinary typed scalar operation selectors. These private
        // names exist only during code generation, never in SemIR or NIR.
        let names = ["$ACTIONC_COMPOUND_LEFT", "$ACTIONC_COMPOUND_RIGHT"];
        let saved = [
            self.local_symbols.insert(names[0].into(), left),
            self.local_symbols.insert(names[1].into(), right),
        ];
        let operand = |name: &str| Expr {
            kind: ExprKind::Name(name.into()),
            text: name.into(),
            span,
        };
        let combined = Expr {
            kind: ExprKind::Binary {
                op,
                left: Box::new(operand(names[0])),
                right: Box::new(operand(names[1])),
            },
            text: String::new(),
            span,
        };
        let emitted = self.emit_expr_to_slot(&combined, result);
        for (name, saved) in names.into_iter().zip(saved) {
            if let Some(saved) = saved {
                self.local_symbols.insert(name.into(), saved);
            } else {
                self.local_symbols.remove(name);
            }
        }
        if !emitted {
            return false;
        }
        self.emit_pla();
        self.emit_sta_zero_page(pointer);
        self.emit_pla();
        self.emit_sta_zero_page(pointer.offset(1));
        self.emit_copy_slot_to_slot(result, destination)
    }

    pub(super) fn index_element(&self, base: &Expr) -> Option<IndexElement> {
        match &base.kind {
            ExprKind::Name(name) => {
                let slot = self.lookup_slot(name)?;
                let size = if slot.array.is_some() {
                    slot.size
                } else {
                    slot.pointee_size?
                };
                Some(IndexElement {
                    size,
                    signed: slot.signed,
                    record: slot.record,
                    is_volatile: slot.is_volatile,
                })
            }
            ExprKind::Field {
                base: record,
                field,
            } => {
                let field = self.record_field_metadata(record, field)?;
                Some(IndexElement {
                    size: field.array?.stride,
                    signed: field.signed,
                    record: field.record,
                    is_volatile: self.expr_side_effect_facts(base).reads_volatile,
                })
            }
            _ => None,
        }
    }

    pub(super) fn field_array_index_slot(
        &mut self,
        base: &Expr,
        index: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let element = self.index_element(base)?;
        let field = self.lvalue_slot(base)?;
        if !self.emit_slot_address(field, pointer)
            || !self.emit_add_scaled_index_to_addr(index, element.size, pointer)
        {
            return None;
        }
        Some(
            StorageSlot::indirect_indexed_y(pointer, element.size)
                .record(element.record)
                .signed(element.signed)
                .volatile(element.is_volatile),
        )
    }

    pub(super) fn emit_dynamic_lvalue_address_to_slot(
        &mut self,
        place: &Expr,
        target: StorageSlot,
    ) -> bool {
        let Some(source) = self.lvalue_slot(place) else {
            return false;
        };
        let pointer = runtime_zp::ARRAY_ADDR;
        if !self.emit_slot_address(source, pointer) {
            return false;
        }
        // Capture both bytes before any target write can alias the address pair.
        self.emit_lda_zero_page_value_only(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page_value_only(pointer);
        self.emit_sta_slot_byte(target, 0);
        self.emit_pla();
        if target.size > 1 {
            self.emit_sta_slot_byte(target, 1);
        }
        true
    }

    // Recursive address preparation can use the canonical scratch pairs even
    // when an outer selector requested another final pointer. Preserve the
    // destination around such expressions using the existing staged fallback.
    pub(super) fn expr_address_needs_nested_scratch(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Index { base, index } => {
                !matches!(base.kind, ExprKind::Name(_))
                    || Self::expr_contains_indirect_lvalue(index)
            }
            ExprKind::Call { callee, args } => {
                !matches!(callee.kind, ExprKind::Name(_))
                    || args.iter().any(Self::expr_contains_indirect_lvalue)
            }
            ExprKind::Field { base, .. } => !matches!(base.kind, ExprKind::Name(_)),
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr,
            } => !matches!(expr.kind, ExprKind::Name(_)),
            ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => {
                Self::expr_address_needs_nested_scratch(expr)
            }
            ExprKind::Binary { left, right, .. } => {
                Self::expr_address_needs_nested_scratch(left)
                    || Self::expr_address_needs_nested_scratch(right)
            }
            _ => false,
        }
    }
}
