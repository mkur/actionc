use super::*;
use crate::semantic::ir::SemCaseArm;

impl NirBuilder {
    pub(super) fn case_statement(&mut self, selector: &SemExpr, arms: &[SemCaseArm]) {
        let value = self.nir_value(selector);
        let after = self.next_block_label();
        self.case_dispatch(value, &selector.ty, arms, &after, |builder, body| {
            builder.stmt_list(body);
            builder.finish_open_goto(&after);
        });
    }

    pub(super) fn case_dispatch<B>(
        &mut self,
        value: NirValue,
        selector_type: &ValueType,
        arms: &[SemCaseArm<B>],
        after: &str,
        mut body: impl FnMut(&mut Self, &B),
    ) {
        let operand_ty = NirType::from_value_with_layout(selector_type, self.target_layout);
        for arm in arms {
            if arm.labels.is_none() && arm.guard.is_none() && arm.tests.is_empty() {
                body(self, &arm.body);
                break;
            }
            let selected = self.next_block_label();
            let next_arm = self.next_block_label();
            let labels = arm.labels.as_deref().unwrap_or(&[]);
            if arm.labels.is_none() {
                self.finish_open_goto(&selected);
            }
            for (index, label) in labels.iter().enumerate() {
                let failed = if index + 1 == labels.len() {
                    next_arm.clone()
                } else {
                    self.next_block_label()
                };
                if label.low == label.high {
                    let condition =
                        self.case_compare(value.clone(), &operand_ty, NirCompareOp::Eq, label.low);
                    self.terminate_branch(condition, &selected, &failed);
                } else {
                    let upper_test = self.next_block_label();
                    let condition =
                        self.case_compare(value.clone(), &operand_ty, NirCompareOp::Ge, label.low);
                    self.terminate_branch(condition, &upper_test, &failed);
                    self.start_block(upper_test);
                    let condition =
                        self.case_compare(value.clone(), &operand_ty, NirCompareOp::Le, label.high);
                    self.terminate_branch(condition, &selected, &failed);
                }
                if index + 1 < labels.len() {
                    self.start_block(failed);
                }
            }
            self.start_block(selected);
            for test in &arm.tests {
                let matched = self.next_block_label();
                self.terminate_condition(test, &matched, &next_arm);
                self.start_block(matched);
            }
            if let Some(guard) = &arm.guard {
                if let Some(bindings) = &guard.bindings {
                    self.stmt_list(&bindings.initialization);
                }
                let accepted = self.next_block_label();
                self.terminate_condition(&guard.condition, &accepted, &next_arm);
                self.start_block(accepted);
            }
            body(self, &arm.body);
            self.start_block(next_arm);
        }
        self.finish_open_goto(after);
        self.start_block(after.to_owned());
    }

    fn case_compare(
        &mut self,
        left: NirValue,
        operand_ty: &NirType,
        op: NirCompareOp,
        bits: u64,
    ) -> NirValue {
        let dest = self.next_temp();
        let ty = NirFacts::condition_type();
        self.push(NirOp::Compare {
            dest,
            ty: ty.clone(),
            operand_ty: operand_ty.clone(),
            op,
            left,
            right: NirValue::IntegerConst {
                bits,
                ty: operand_ty.kind.integer().expect("integer CASE selector"),
            },
        });
        NirValue::Temp { id: dest, ty }
    }
}
