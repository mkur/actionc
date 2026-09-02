use std::collections::{BTreeMap, BTreeSet};

use super::{collect_routine_temp_widths, op_def};
use crate::mir6502::analysis::effects::{classify_op, classify_terminator};
use crate::mir6502::ir::{
    MirBinaryOp, MirCarryOut, MirDef, MirOp, MirRoutine, MirTempId, MirValue, MirWidth,
};

#[derive(Debug, Default)]
pub(super) struct NarrowProductStats {
    pub candidates: usize,
    pub applied: usize,
    pub blocked_high_lane_live: usize,
    pub blocked_multiple_definitions: usize,
    pub blocked_operand_width: usize,
    pub blocked_carry_contract: usize,
    pub low_only_results: BTreeSet<MirTempId>,
}

#[derive(Debug, Default)]
struct TempFacts {
    definitions: usize,
    uses: usize,
}

pub(super) fn narrow_discarded_high_constant_products(
    routine: &mut MirRoutine,
) -> NarrowProductStats {
    let widths = collect_routine_temp_widths(routine);
    let facts = collect_temp_facts(routine);
    let mut stats = NarrowProductStats::default();

    for block in &mut routine.blocks {
        let ops = std::mem::take(&mut block.ops);
        let mut out = Vec::with_capacity(ops.len());
        let mut index = 0usize;
        while index < ops.len() {
            let MirOp::Binary {
                op: MirBinaryOp::Mul,
                dst,
                left,
                right,
                width: MirWidth::Word,
                carry_in,
                carry_out,
            } = &ops[index]
            else {
                out.push(ops[index].clone());
                index += 1;
                continue;
            };

            let Some((operand, factor)) = narrow_constant_multiply_parts(left, right) else {
                out.push(ops[index].clone());
                index += 1;
                continue;
            };
            stats.candidates += 1;

            if carry_in.is_some() || *carry_out != MirCarryOut::Ignore {
                stats.blocked_carry_contract += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            }
            if !value_is_proven_byte(&operand, &widths) {
                stats.blocked_operand_width += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            }
            let MirDef::VTemp(product) = dst else {
                stats.blocked_multiple_definitions += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            };
            let Some(product_facts) = facts.get(product) else {
                stats.blocked_multiple_definitions += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            };
            if product_facts.definitions != 1 {
                stats.blocked_multiple_definitions += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            }
            if product_facts.uses != 1 {
                stats.blocked_high_lane_live += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            }

            let Some(MirOp::Truncate {
                dst: truncated,
                src: MirValue::Def(MirDef::VTemp(source)),
                from_width: MirWidth::Word,
                to_width: MirWidth::Byte,
            }) = ops.get(index + 1)
            else {
                stats.blocked_high_lane_live += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            };
            if source != product {
                stats.blocked_high_lane_live += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            }
            let MirDef::VTemp(low_only_result) = truncated else {
                stats.blocked_high_lane_live += 1;
                out.push(ops[index].clone());
                index += 1;
                continue;
            };

            out.push(MirOp::Binary {
                op: MirBinaryOp::Mul,
                dst: truncated.clone(),
                left: operand,
                right: MirValue::ConstU8(factor),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            });
            stats.low_only_results.insert(*low_only_result);
            stats.applied += 1;
            index += 2;
        }
        block.ops = out;
    }

    stats
}

fn narrow_constant_multiply_parts(left: &MirValue, right: &MirValue) -> Option<(MirValue, u8)> {
    let (operand, factor) = if let Some(factor) = constant_u16(right) {
        (left.clone(), factor)
    } else {
        (right.clone(), constant_u16(left)?)
    };
    (factor > 1 && factor <= u16::from(u8::MAX) && factor.is_power_of_two())
        .then_some((operand, factor as u8))
}

fn constant_u16(value: &MirValue) -> Option<u16> {
    match value {
        MirValue::ConstU8(value) => Some(u16::from(*value)),
        MirValue::ConstU16(value) => Some(*value),
        _ => None,
    }
}

fn value_is_proven_byte(value: &MirValue, widths: &BTreeMap<MirTempId, MirWidth>) -> bool {
    match value {
        MirValue::ConstU8(_) | MirValue::Def(MirDef::VTempByte { .. }) => true,
        MirValue::Def(MirDef::VTemp(id)) => widths.get(id) == Some(&MirWidth::Byte),
        MirValue::ConstU16(_)
        | MirValue::Def(MirDef::Reg(_))
        | MirValue::PointerCell(_)
        | MirValue::Word { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::GlobalAddr(_)
        | MirValue::StaticAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => false,
    }
}

fn collect_temp_facts(routine: &MirRoutine) -> BTreeMap<MirTempId, TempFacts> {
    let mut facts = BTreeMap::<MirTempId, TempFacts>::new();
    for block in &routine.blocks {
        for op in &block.ops {
            if let Some(def) = op_def(op) {
                let id = match def {
                    MirDef::VTemp(id) | MirDef::VTempByte { id, .. } => Some(*id),
                    MirDef::Reg(_) => None,
                };
                if let Some(id) = id {
                    facts.entry(id).or_default().definitions += 1;
                }
            }
            for access in &classify_op(op).logical.temp_uses {
                facts.entry(access.temp()).or_default().uses += 1;
            }
        }
        for access in &classify_terminator(&block.terminator).logical.temp_uses {
            facts.entry(access.temp()).or_default().uses += 1;
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirBlockId, MirEffects, MirFrame, MirMem, MirRoutineAbi, MirTemp,
        MirTerminator, RoutineId,
    };
    use crate::nir::SymbolId;

    fn product_routine(extra_use: Option<MirOp>, operand: MirValue) -> MirRoutine {
        let mut ops = vec![
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(0)),
                src: MirAddr::Direct(MirMem::Global {
                    id: SymbolId(0),
                    offset: 0,
                }),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Mul,
                dst: MirDef::VTemp(MirTempId(1)),
                left: MirValue::ConstU16(32),
                right: operand,
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Truncate {
                dst: MirDef::VTemp(MirTempId(2)),
                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                from_width: MirWidth::Word,
                to_width: MirWidth::Byte,
            },
        ];
        if let Some(extra_use) = extra_use {
            ops.push(extra_use);
        }
        MirRoutine {
            id: RoutineId(0),
            name: "Product".into(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: (0..=2).map(|id| MirTemp { id: MirTempId(id) }).collect(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".into(),
                params: Vec::new(),
                ops,
                terminator: MirTerminator::Return,
            }],
            effects: MirEffects::default(),
        }
    }

    #[test]
    fn sole_low_byte_product_use_is_narrowed() {
        let mut routine = product_routine(None, MirValue::Def(MirDef::VTemp(MirTempId(0))));
        let stats = narrow_discarded_high_constant_products(&mut routine);

        assert_eq!(stats.applied, 1);
        assert_eq!(routine.blocks[0].ops.len(), 2);
        assert!(matches!(
            &routine.blocks[0].ops[1],
            MirOp::Binary {
                op: MirBinaryOp::Mul,
                dst: MirDef::VTemp(MirTempId(2)),
                right: MirValue::ConstU8(32),
                width: MirWidth::Byte,
                ..
            }
        ));
    }

    #[test]
    fn full_word_product_use_blocks_narrowing() {
        let extra_use = MirOp::Store {
            dst: MirAddr::Direct(MirMem::Global {
                id: SymbolId(1),
                offset: 0,
            }),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Word,
        };
        let mut routine =
            product_routine(Some(extra_use), MirValue::Def(MirDef::VTemp(MirTempId(0))));
        let stats = narrow_discarded_high_constant_products(&mut routine);

        assert_eq!(stats.applied, 0);
        assert_eq!(stats.blocked_high_lane_live, 1);
        assert!(matches!(
            routine.blocks[0].ops[1],
            MirOp::Binary {
                width: MirWidth::Word,
                ..
            }
        ));
    }

    #[test]
    fn pointer_cell_operand_keeps_the_word_path() {
        let mut routine = product_routine(
            None,
            MirValue::PointerCell(MirMem::Global {
                id: SymbolId(0),
                offset: 0,
            }),
        );
        let stats = narrow_discarded_high_constant_products(&mut routine);

        assert_eq!(stats.applied, 0);
        assert_eq!(stats.blocked_operand_width, 1);
    }

    #[test]
    fn low_byte_products_match_word_products_for_every_byte() {
        for factor in [2u16, 4, 32, 128] {
            for value in 0..=u8::MAX {
                let word = factor.wrapping_mul(u16::from(value));
                let byte = (factor as u8).wrapping_mul(value);
                assert_eq!(byte, word as u8);
            }
        }
    }

    #[test]
    fn low_only_proof_selects_byte_strength_reduction() {
        let dst = MirDef::VTemp(MirTempId(2));
        let proven = BTreeSet::from([MirTempId(2)]);
        assert_eq!(
            super::super::strength_reduced_multiply_width(MirWidth::Byte, &dst, &proven),
            MirWidth::Byte
        );
        assert_eq!(
            super::super::strength_reduced_multiply_width(MirWidth::Byte, &dst, &BTreeSet::new()),
            MirWidth::Word
        );
    }
}
