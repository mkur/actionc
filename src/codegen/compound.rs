//! Shared scalar compound-operation facts and captured-place lowering.
use super::*;

#[derive(Debug, Clone, Default)]
pub(super) struct ClassicCompoundFacts {
    statements: HashMap<(String, usize, usize), crate::semantic::ir::SemCompoundOperation>,
}

impl ClassicCompoundFacts {
    pub(super) fn extend(&mut self, other: Self) {
        self.statements.extend(other.statements);
    }

    pub(super) fn insert(
        &mut self,
        scope: Option<&str>,
        span: Span,
        operation: crate::semantic::ir::SemCompoundOperation,
    ) {
        self.statements.insert(
            (scope.unwrap_or_default().to_owned(), span.start, span.end),
            operation,
        );
    }

    pub(super) fn operation(
        &self,
        scope: Option<&str>,
        span: Span,
    ) -> Option<&crate::semantic::ir::SemCompoundOperation> {
        self.statements
            .get(&(scope.unwrap_or_default().to_owned(), span.start, span.end))
    }
}

impl Generator {
    pub(super) fn emit_captured_compound_assignment(
        &mut self,
        target: &Expr,
        op: BinaryOp,
        value: &Expr,
        span: Span,
    ) -> bool {
        let Some(slot) = self.compatible_compound_target_slot(target) else {
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
        let right = StorageSlot::zero_page(
            runtime_zp::ARGS.address(),
            self.expr_size(value).unwrap_or(slot.size),
        )
        .signed(self.expr_signed(value));
        if !self.emit_expr_to_slot(value, right) {
            return false;
        }
        // The cartridge captures the address before RHS effects, but reads
        // its value afterwards. Restore the pointer, then keep it safe across
        // arithmetic helpers as well.
        self.emit_pla();
        self.emit_sta_zero_page(pointer);
        self.emit_pla();
        self.emit_sta_zero_page(pointer.offset(1));
        self.emit_lda_zero_page_value_only(pointer.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page_value_only(pointer);
        self.emitter.emit_pha();
        let left =
            StorageSlot::zero_page(runtime_zp::VALUE_TEMP.address(), slot.size).signed(slot.signed);
        for byte in 0..slot.size {
            self.emit_lda_slot_byte(destination, byte);
            self.emit_sta_slot_byte(left, byte);
        }
        let operation_type = self
            .compound_operations
            .operation(self.current_record_copy_scope.as_deref(), span)
            .map(|operation| &operation.result_type);
        let result_size = operation_type
            .and_then(|ty| ty.as_scalar())
            .map_or(slot.size, |ty| ty.width_bytes());
        let result_signed = operation_type
            .and_then(|ty| ty.as_scalar())
            .map_or(slot.signed, |ty| {
                ty.signedness() == crate::semantic::ScalarSignedness::Signed
            });
        let result = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), result_size)
            .signed(result_signed);
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
}
