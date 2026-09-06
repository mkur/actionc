use super::*;
use crate::semantic::ir::SemCaseArm;

impl NirBuilder {
    pub(super) fn case_statement(
        &mut self,
        selector: &SemExpr,
        arms: &[SemCaseArm],
        lowering: &mut NirLowerer,
    ) {
        let value = self.nir_value(selector);
        let operand_ty = NirFacts::type_from_value(&selector.ty);
        let after = lowering.next_block_label();
        for arm in arms {
            let Some(labels) = &arm.labels else {
                self.stmt_list(&arm.body, lowering);
                self.finish_open_goto(&after);
                break;
            };
            let body = lowering.next_block_label();
            let next_arm = lowering.next_block_label();
            for (index, label) in labels.iter().enumerate() {
                let failed = if index + 1 == labels.len() {
                    next_arm.clone()
                } else {
                    lowering.next_block_label()
                };
                if label.low == label.high {
                    let condition =
                        self.case_compare(value.clone(), &operand_ty, NirCompareOp::Eq, label.low);
                    self.terminate_branch(condition, &body, &failed);
                } else {
                    let upper_test = lowering.next_block_label();
                    let condition =
                        self.case_compare(value.clone(), &operand_ty, NirCompareOp::Ge, label.low);
                    self.terminate_branch(condition, &upper_test, &failed);
                    self.start_block(upper_test);
                    let condition =
                        self.case_compare(value.clone(), &operand_ty, NirCompareOp::Le, label.high);
                    self.terminate_branch(condition, &body, &failed);
                }
                if index + 1 < labels.len() {
                    self.start_block(failed);
                }
            }
            self.start_block(body);
            self.stmt_list(&arm.body, lowering);
            self.finish_open_goto(&after);
            self.start_block(next_arm);
        }
        self.finish_open_goto(&after);
        self.start_block(after);
    }

    fn case_compare(
        &mut self,
        left: NirValue,
        operand_ty: &NirType,
        op: NirCompareOp,
        bits: u16,
    ) -> NirValue {
        let dest = self.next_temp();
        let ty = NirFacts::condition_type();
        self.push(NirOp::Compare {
            dest,
            ty: ty.clone(),
            operand_ty: operand_ty.clone(),
            op,
            left,
            right: nir_scalar_constant(operand_ty, bits),
        });
        NirValue::Temp { id: dest, ty }
    }
}
