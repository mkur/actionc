//! Reuse and loop transforms for typed index arithmetic, never memory values.
use super::*;

fn arithmetic_inputs(op: &NirOp) -> Option<Vec<&NirValue>> {
    match op {
        NirOp::Cast { src, from, to, .. }
            if from.kind.integer().is_some() && to.kind.integer().is_some() =>
        {
            Some(vec![src])
        }
        NirOp::Unary { src, ty, .. } if ty.kind.integer().is_some() => Some(vec![src]),
        NirOp::Binary {
            left,
            right,
            op: NirBinaryOp::Add | NirBinaryOp::Sub | NirBinaryOp::Mul,
            ty,
            ..
        } if ty.kind.integer().is_some() => Some(vec![left, right]),
        _ => None,
    }
}

fn index_temps(routine: &NirRoutine) -> BTreeSet<TempId> {
    fn place_seeds(place: &NirPlace, pending: &mut Vec<TempId>) {
        match &place.kind {
            NirPlaceKind::Index {
                index: NirValue::Temp { id, .. },
                ..
            } => pending.push(*id),
            NirPlaceKind::Field { base, .. } => place_seeds(base, pending),
            _ => {}
        }
    }
    let mut pending = Vec::new();
    let mut definitions = BTreeMap::new();
    for op in routine.blocks.iter().flat_map(|b| &b.ops) {
        if let Some((dest, _)) = op_def(op) {
            definitions.insert(dest, op);
        }
        match op {
            NirOp::Load { place, .. }
            | NirOp::Store { place, .. }
            | NirOp::VolatileLoad { place, .. }
            | NirOp::VolatileStore { place, .. }
            | NirOp::AddrOf { place, .. } => place_seeds(place, &mut pending),
            NirOp::CopyBytes {
                source,
                destination,
                ..
            } => {
                place_seeds(source, &mut pending);
                place_seeds(destination, &mut pending);
            }
            NirOp::Call {
                aggregate_result: Some(place),
                ..
            } => place_seeds(place, &mut pending),
            _ => {}
        }
    }
    let mut result = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !result.insert(id) {
            continue;
        }
        if let Some(inputs) = definitions.get(&id).and_then(|op| arithmetic_inputs(op)) {
            for value in inputs {
                if let NirValue::Temp { id, .. } = value {
                    pending.push(*id);
                }
            }
        }
    }
    result
}

fn ordering_barrier(op: &NirOp) -> bool {
    matches!(
        op,
        NirOp::Call { .. }
            | NirOp::ForeignCode { .. }
            | NirOp::VolatileLoad { .. }
            | NirOp::VolatileStore { .. }
            | NirOp::Real(_)
    ) || matches!(
        op,
        NirOp::CopyBytes {
            source_volatile: true,
            ..
        } | NirOp::CopyBytes {
            destination_volatile: true,
            ..
        }
    )
}

pub(super) fn reuse_local_arithmetic(routine: &mut NirRoutine) {
    let indexes = index_temps(routine);
    let mut replacements = BTreeMap::new();
    for block in &mut routine.blocks {
        let mut available = Vec::<(PureExpression, NirValue, usize)>::new();
        let mut retained = Vec::new();
        for (position, mut op) in std::mem::take(&mut block.ops).into_iter().enumerate() {
            rewrite_op_values(&mut op, &replacements);
            if ordering_barrier(&op) {
                available.clear();
            }
            if let Some((dest, ty)) = op_def(&op)
                && indexes.contains(&dest)
                && arithmetic_inputs(&op).is_some()
                && let Some(expression) = pure_expression(&op)
            {
                // Bound extra lifetime to a small region of one block. General
                // GVN keeps its stricter no-live-range-extension policy.
                available.retain(|(_, _, at)| position - at <= 64);
                if let Some((_, value, _)) =
                    available.iter().rev().find(|(e, _, _)| e == &expression)
                {
                    replacements.insert(dest, value.clone());
                    continue;
                }
                available.push((
                    expression,
                    NirValue::Temp {
                        id: dest,
                        ty: ty.clone(),
                    },
                    position,
                ));
            }
            retained.push(op);
        }
        block.ops = retained;
    }
    // A removed definition may also feed successor blocks or edge arguments.
    for block in &mut routine.blocks {
        for op in &mut block.ops {
            rewrite_op_values(op, &replacements);
        }
        rewrite_terminator_values(&mut block.terminator, &replacements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(source: &str) -> NirProgram {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))
    }

    // Give two real indexed stores the same typed, already-captured product.
    // This tests the pass directly, without constant folding or memory forwarding
    // masking whether its bounded reuse and effect barrier actually work.
    fn duplicate_products(middle: &str) -> NirProgram {
        let mut p = program(&format!(
            "CARD ARRAY a(20)\nPROC Side() RETURN\nPROC Main() a(1)=1 {middle} a(1)=2 RETURN"
        ));
        let r = p.routines.last_mut().unwrap();
        let ty = NirType::from_value(&crate::semantic::ValueType::scalar(
            crate::semantic::ScalarType::Int,
        ));
        let mut next = r.temps.iter().map(|t| t.id.0).max().unwrap_or(0) + 1;
        for b in &mut r.blocks {
            let mut ops = Vec::new();
            for mut op in std::mem::take(&mut b.ops) {
                if let NirOp::Store {
                    place:
                        NirPlace {
                            kind: NirPlaceKind::Index { index, .. },
                            ..
                        },
                    ..
                } = &mut op
                {
                    let dest = TempId(next);
                    next += 1;
                    ops.push(NirOp::Binary {
                        dest,
                        ty: ty.clone(),
                        op: NirBinaryOp::Mul,
                        left: NirValue::ConstU16(2),
                        right: NirValue::ConstU16(3),
                    });
                    *index = NirValue::Temp {
                        id: dest,
                        ty: ty.clone(),
                    };
                }
                ops.push(op);
            }
            b.ops = ops;
        }
        r.temps = collect_temps(&r.blocks);
        verify_program(&p).unwrap();
        p
    }

    #[test]
    fn index_reuse_extends_only_bounded_local_arithmetic_lifetimes() {
        let mut p = duplicate_products("");
        let r = p.routines.last_mut().unwrap();
        reuse_local_arithmetic(r);
        r.temps = collect_temps(&r.blocks);
        assert_eq!(
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(
                    op,
                    NirOp::Binary {
                        op: NirBinaryOp::Mul,
                        ..
                    }
                ))
                .count(),
            1
        );
        verify_program(&p).unwrap();
    }

    #[test]
    fn index_reuse_keeps_calls_as_region_boundaries() {
        let mut p = duplicate_products("Side()");
        let r = p.routines.last_mut().unwrap();
        reuse_local_arithmetic(r);
        r.temps = collect_temps(&r.blocks);
        assert_eq!(
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(
                    op,
                    NirOp::Binary {
                        op: NirBinaryOp::Mul,
                        ..
                    }
                ))
                .count(),
            2
        );
        verify_program(&p).unwrap();
    }
}
