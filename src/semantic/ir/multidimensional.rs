//! Shared typed row-major computation for NIR and the classic projection.
use super::*;

impl SemMultiIndex {
    /// The caller captures `base` before evaluating this expression. Its left
    /// spine evaluates each coordinate exactly once, in source order. ADDRESS
    /// arithmetic is compiler-generated here; source coordinate arithmetic has
    /// already been typed and finishes before the widening conversion.
    pub(crate) fn normalized_index(&self) -> SemExpr {
        assert!(self.shape.rank() > 1);
        assert_eq!(self.coordinates.len(), self.shape.rank());
        let ty = ValueType::scalar(ScalarType::Address);
        let expression = |kind, span| SemExpr {
            kind,
            ty: ty.clone(),
            class: SemExprClass::Value,
            eval_order: None,
            span,
        };
        let cast = |coordinate: &SemExpr| {
            assert!(
                coordinate.ty.as_scalar().is_some(),
                "checked integer coordinate"
            );
            expression(
                SemExprKind::Cast {
                    ty: ty.clone(),
                    expr: Box::new(coordinate.clone()),
                },
                coordinate.span,
            )
        };
        let mut linear = cast(&self.coordinates[0]);
        for (coordinate, &dimension) in self.coordinates.iter().zip(self.shape.dimensions()).skip(1)
        {
            let dimension = expression(
                SemExprKind::Literal(SemLiteral::Constant(ConstValue {
                    ty: ScalarType::Address,
                    bits: u64::from(dimension),
                })),
                coordinate.span,
            );
            let product = expression(
                SemExprKind::Binary {
                    op: BinaryOp::Mul,
                    left: Box::new(linear),
                    right: Box::new(dimension),
                },
                coordinate.span,
            );
            linear = expression(
                SemExprKind::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(product),
                    right: Box::new(cast(coordinate)),
                },
                coordinate.span,
            );
        }
        linear
    }
}
