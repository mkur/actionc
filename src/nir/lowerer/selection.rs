use super::*;
use crate::semantic::ir::{SemCaseResult, SemCaseValue, SemIfValue};

impl NirBuilder {
    pub(super) fn case_value(&mut self, selection: &SemCaseValue, ty: &ValueType) -> NirValue {
        self.stmt_list(&selection.preparation);
        let selector = self.nir_value(&selection.selector);
        let join = self.next_block_label();
        let target = self.block_id_for_label(&join);
        self.case_dispatch(
            selector,
            &selection.selector.ty,
            &selection.arms,
            &join,
            |builder, result| match result {
                SemCaseResult::Fault { kind, span } => builder.stmt(&SemStmt::Fault {
                    kind: *kind,
                    span: *span,
                }),
                SemCaseResult::Yield { preparation, value } => {
                    builder.stmt_list(preparation);
                    let value = builder.selection_result_value(value);
                    builder.terminate(NirTerminator::Goto(NirEdge {
                        target,
                        args: vec![value],
                    }));
                }
            },
        );
        self.value_join_result(ty)
    }

    pub(super) fn if_value(&mut self, selection: &SemIfValue, ty: &ValueType) -> NirValue {
        let join = self.next_block_label();
        let target = self.block_id_for_label(&join);
        for (condition, value) in &selection.branches {
            let selected = self.next_block_label();
            let next = self.next_block_label();
            self.terminate_condition(condition, &selected, &next);
            self.start_block(selected);
            let value = self.selection_result_value(value);
            self.terminate(NirTerminator::Goto(NirEdge {
                target,
                args: vec![value],
            }));
            self.start_block(next);
        }
        let value = self.selection_result_value(&selection.otherwise);
        self.terminate(NirTerminator::Goto(NirEdge {
            target,
            args: vec![value],
        }));
        self.start_block(join);
        self.value_join_result(ty)
    }

    fn selection_result_value(&mut self, expression: &SemExpr) -> NirValue {
        let value = self.nir_value(expression);
        // Comparisons produce NIR Bool. Action! comparison values are BYTE,
        // so materialize that representation before supplying a typed edge.
        let ty = NirType::from_value_with_layout(&expression.ty, self.target_layout);
        self.convert_integer_operation_input(value, &ty)
    }

    fn value_join_result(&mut self, ty: &ValueType) -> NirValue {
        let ty = NirType::from_value_with_layout(ty, self.target_layout);
        let dest = self.next_temp();
        self.blocks[self.current].params.push(NirBlockParam {
            dest,
            ty: ty.clone(),
        });
        NirValue::Temp { id: dest, ty }
    }
}
