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
