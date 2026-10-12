//! Address-value provenance from the generation-scoped word-flow context.
#![allow(dead_code)] // Selection is enabled by the subsequent vertical slices.
use super::effects::classify_op;
use super::sites::MirSite;
use crate::mir6502::ir::*;
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct AddressValueFact {
    pub base: MirValue,
    pub index: Option<MirValue>,
    pub elem_size: u16,
    pub offset: u16,
    pub producers: BTreeSet<usize>,
    /// Original input-use sites; availability must be proved at a new consumer.
    pub inputs: Vec<(MirValue, MirSite)>,
}

pub(in crate::mir6502) fn address_value(
    block: &MirBlock,
    value: &MirValue,
    site: MirSite,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<AddressValueFact> {
    trace(block, value, site, context, 32)
}

fn leaf(value: &MirValue, site: MirSite) -> Option<AddressValueFact> {
    fn captured(v: &MirValue) -> bool {
        match v {
            MirValue::PointerCell(_) | MirValue::Def(MirDef::Reg(_)) => false,
            MirValue::Word { lo, hi } => captured(lo) && captured(hi),
            _ => true,
        }
    }
    captured(value).then(|| AddressValueFact {
        base: value.clone(),
        index: None,
        elem_size: 1,
        offset: 0,
        producers: BTreeSet::new(),
        inputs: vec![(value.clone(), site)],
    })
}

fn defining_op<'a>(
    block: &'a MirBlock,
    value: &MirValue,
    site: MirSite,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<(usize, &'a MirOp)> {
    let MirValue::Def(MirDef::VTemp(temp)) = value else {
        return None;
    };
    let MirProof::Proven(defs) = context.exclusive_word_temp_definition(*temp, site) else {
        return None;
    };
    let MirSite::Op {
        block: id,
        op_index,
    } = defs[0].site
    else {
        return None;
    };
    (id == block.id).then(|| (op_index, &block.ops[op_index]))
}

fn constant_word(
    block: &MirBlock,
    value: &MirValue,
    site: MirSite,
    context: &PreHomeRewriteContext<'_, '_>,
    depth: usize,
) -> Option<(u16, BTreeSet<usize>)> {
    match value {
        MirValue::ConstU8(v) => return Some((u16::from(*v), BTreeSet::new())),
        MirValue::ConstU16(v) => return Some((*v, BTreeSet::new())),
        _ => {}
    }
    if depth == 0 {
        return None;
    }
    let (i, op) = defining_op(block, value, site, context)?;
    let original = MirSite::Op {
        block: block.id,
        op_index: i,
    };
    let (v, mut removed) = match op {
        MirOp::Move {
            src,
            width: MirWidth::Word,
            ..
        } => constant_word(block, src, original, context, depth - 1)?,
        MirOp::Extend {
            src: MirValue::ConstU8(v),
            from_width: MirWidth::Byte,
            to_width: MirWidth::Word,
            signed: false,
            ..
        } => (u16::from(*v), BTreeSet::new()),
        MirOp::LoadImm {
            value,
            width: MirWidth::Word,
            ..
        } => (*value, BTreeSet::new()),
        _ => return None,
    };
    removed.insert(i);
    Some((v, removed))
}

fn trace(
    block: &MirBlock,
    value: &MirValue,
    site: MirSite,
    context: &PreHomeRewriteContext<'_, '_>,
    depth: usize,
) -> Option<AddressValueFact> {
    if depth == 0 {
        return None;
    }
    let Some((i, op)) = defining_op(block, value, site, context) else {
        return leaf(value, site);
    };
    let original = MirSite::Op {
        block: block.id,
        op_index: i,
    };
    let mut fact = match op {
        MirOp::Move {
            src,
            width: MirWidth::Word,
            ..
        } => trace(block, src, original, context, depth - 1)?,
        MirOp::Binary {
            op: MirBinaryOp::Add,
            left,
            right: MirValue::ConstU16(offset),
            width: MirWidth::Word,
            carry_in: None | Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Ignore,
            ..
        } => {
            let mut f = trace(block, left, original, context, depth - 1)?;
            f.offset = f.offset.wrapping_add(*offset);
            f
        }
        MirOp::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            width: MirWidth::Word,
            carry_in: None | Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Ignore,
            ..
        } => {
            let Some((mul, scaled)) = defining_op(block, right, original, context) else {
                return leaf(value, site);
            };
            let MirOp::Binary {
                op: MirBinaryOp::Mul,
                left: index,
                right: MirValue::ConstU16(stride),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
                ..
            } = scaled
            else {
                return leaf(value, site);
            };
            let mul_site = MirSite::Op {
                block: block.id,
                op_index: mul,
            };
            let mut f = trace(block, left, original, context, depth - 1)?;
            if f.index.is_some() {
                return None;
            }
            if let Some((constant, removed)) =
                constant_word(block, index, mul_site, context, depth - 1)
            {
                f.offset = f.offset.wrapping_add(constant.wrapping_mul(*stride));
                f.producers.extend(removed);
            } else {
                leaf(index, mul_site)?;
                f.index = Some(index.clone());
                f.inputs.push((index.clone(), mul_site));
            }
            f.elem_size = *stride;
            f.producers.insert(mul);
            f
        }
        _ => return leaf(value, site),
    };
    fact.producers.insert(i);
    Some(fact)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum PointerStoreBlocker {
    NotLocal,
    NoStore,
    Barrier(usize),
}

/// Logical reaching-store fact only. Physical noninterference is a later proof.
pub(in crate::mir6502) fn local_pointer_store(
    block: &MirBlock,
    end: usize,
    cell: &MirMem,
) -> Result<(usize, MirValue), PointerStoreBlocker> {
    if !matches!(cell, MirMem::Local { .. }) {
        return Err(PointerStoreBlocker::NotLocal);
    }
    for i in (0..end).rev() {
        if let MirOp::Store {
            dst: MirAddr::Direct(dst),
            src,
            width: MirWidth::Word,
        } = &block.ops[i]
        {
            if dst == cell {
                return Ok((i, src.clone()));
            }
        }
        let e = classify_op(&block.ops[i]);
        if e.memory.may_write_any
            || e.memory.has_unknown_effects
            || !e.memory.direct_writes.is_empty()
            || !matches!(e.memory.structured_writes, MirMemoryEffect::None)
            || matches!(
                block.ops[i],
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            )
        {
            return Err(PointerStoreBlocker::Barrier(i));
        }
    }
    Err(PointerStoreBlocker::NoStore)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    fn lower(source: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }
    #[test]
    fn address_value_proves_full_index_and_zero_extension_without_loading_again() {
        for index in ["index", "0"] {
            let p = lower(&format!(
                "TYPE E=[CARD value BYTE ARRAY pad(18)] TYPE T=[BYTE ARRAY pad(38) E ARRAY rows(2)] T POINTER p CARD index,result PROC Main() result=@p.rows({index}) RETURN"
            ));
            let r = &p.routines[0];
            let b = &r.blocks[0];
            let snapshot =
                PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let ctx = PreHomeRewriteContext::new(&snapshot);
            let (end, src) = b
                .ops
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, op)| {
                    if let MirOp::Store {
                        src,
                        width: MirWidth::Word,
                        ..
                    } = op
                    {
                        Some((i, src))
                    } else {
                        None
                    }
                })
                .unwrap();
            let f = address_value(
                b,
                src,
                MirSite::Op {
                    block: b.id,
                    op_index: end,
                },
                &ctx,
            )
            .unwrap();
            assert_eq!(f.offset, 38);
            assert_eq!(f.elem_size, 20);
            assert_eq!(f.index.is_some(), index == "index");
            assert!(
                f.producers
                    .iter()
                    .all(|i| !matches!(b.ops[*i], MirOp::Load { .. } | MirOp::Call { .. }))
            );
        }
    }
    #[test]
    fn address_value_does_not_remove_a_scaled_value_with_an_extra_lane_use() {
        let mut p = lower(
            "TYPE E=[CARD value BYTE ARRAY pad(18)] TYPE T=[BYTE ARRAY pad(38) E ARRAY rows(2)] T POINTER p CARD index,result PROC Main() result=@p.rows(index) RETURN",
        );
        let r = &mut p.routines[0];
        let scaled = r.blocks[0]
            .ops
            .iter()
            .find_map(|op| {
                if let MirOp::Binary {
                    op: MirBinaryOp::Mul,
                    dst: MirDef::VTemp(id),
                    ..
                } = op
                {
                    Some(*id)
                } else {
                    None
                }
            })
            .unwrap();
        let new = MirTempId(r.temps.iter().map(|t| t.id.0).max().unwrap() + 1);
        r.temps.push(MirTemp { id: new });
        r.blocks[0].ops.push(MirOp::Move {
            dst: MirDef::VTemp(new),
            src: MirValue::Def(MirDef::VTempByte {
                id: scaled,
                byte: 1,
            }),
            width: MirWidth::Byte,
        });
        let b = &r.blocks[0];
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let ctx = PreHomeRewriteContext::new(&snapshot);
        let (end, src) = b
            .ops
            .iter()
            .enumerate()
            .find_map(|(i, op)| {
                if let MirOp::Store {
                    src,
                    width: MirWidth::Word,
                    ..
                } = op
                {
                    Some((i, src))
                } else {
                    None
                }
            })
            .unwrap();
        let fact = address_value(
            b,
            src,
            MirSite::Op {
                block: b.id,
                op_index: end,
            },
            &ctx,
        )
        .unwrap();
        assert!(fact.index.is_none());
        assert!(!fact.producers.iter().any(|i| matches!(
            b.ops[*i],
            MirOp::Binary {
                op: MirBinaryOp::Mul,
                ..
            }
        )));
    }
    #[test]
    fn local_store_provenance_rejects_mutation_calls_and_nonlocal_cells() {
        let p = lower(
            "TYPE E=[CARD value] E object PROC Main() E POINTER p p=@object p.value=7 RETURN",
        );
        let mut b = p.routines[0].blocks[0].clone();
        let (store, cell) = b
            .ops
            .iter()
            .enumerate()
            .find_map(|(i, op)| {
                if let MirOp::Store {
                    dst: MirAddr::Direct(cell @ MirMem::Local { .. }),
                    width: MirWidth::Word,
                    ..
                } = op
                {
                    Some((i, cell.clone()))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(local_pointer_store(&b, store + 1, &cell).unwrap().0, store);
        b.ops.insert(
            store + 1,
            MirOp::Store {
                dst: MirAddr::Direct(cell.clone()),
                src: MirValue::ConstU8(0),
                width: MirWidth::Byte,
            },
        );
        assert_eq!(
            local_pointer_store(&b, store + 2, &cell),
            Err(PointerStoreBlocker::Barrier(store + 1))
        );
        assert_eq!(
            local_pointer_store(&b, store + 1, &MirMem::Absolute(0x600)),
            Err(PointerStoreBlocker::NotLocal)
        );
        let calls = lower(
            "TYPE E=[CARD value] E object PROC Touch() RETURN PROC Main() E POINTER p p=@object Touch() p.value=7 RETURN",
        );
        let cb = &calls
            .routines
            .iter()
            .find(|r| r.name == "Main")
            .unwrap()
            .blocks[0];
        let call = cb
            .ops
            .iter()
            .position(|op| matches!(op, MirOp::Call { .. }))
            .unwrap();
        let cc = cb
            .ops
            .iter()
            .find_map(|op| {
                if let MirOp::Store {
                    dst: MirAddr::Direct(cell @ MirMem::Local { .. }),
                    width: MirWidth::Word,
                    ..
                } = op
                {
                    Some(cell)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            local_pointer_store(cb, call + 1, cc),
            Err(PointerStoreBlocker::Barrier(call))
        );
    }
}
