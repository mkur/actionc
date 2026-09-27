//! NULL acquires a pointer type at the semantic boundary. Backends see only
//! the existing typed zero cast, never an untyped sentinel or source spelling.
use super::*;

impl Analyzer {
    pub(super) fn is_builtin_null_name(&self, scope: ScopeId, name: &str) -> bool {
        name.eq_ignore_ascii_case("NULL")
            && self.lookup_symbol(scope, name).is_none()
            && self.opened_variant_constructor(scope, name).is_none()
    }

    pub(super) fn is_builtin_null(&self, scope: ScopeId, expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::Name(name) if self.is_builtin_null_name(scope, name))
    }

    pub(super) fn lower_null(
        &mut self,
        scope: ScopeId,
        span: Span,
        expected: Option<&ValueType>,
    ) -> subject::SemExpr {
        if !self.options.null_values {
            self.diagnostics.push(Diagnostic::new(span, "NULL requires the modern profile"));
            return self.error_expr(span);
        }
        let Some(ty) = expected.filter(|ty| ty.is_pointer() || ty.as_callable_pointer().is_some()) else {
            self.diagnostics.push(Diagnostic::new(span,
                "NULL requires a pointer context: an assignment, argument, return, explicit pointer cast or equality comparison"));
            return self.error_expr(span);
        };
        self.null_pointer_types.insert(ExpressionSite::new(scope, span), ty.clone());
        subject::SemExpr {
            ty: ty.clone(),
            kind: subject::SemExprKind::Cast {
                ty: ty.clone(),
                expr: Box::new(subject::SemExpr {
                    ty: ValueType::scalar(ScalarType::Byte),
                    kind: subject::SemExprKind::Literal(subject::SemLiteral::Number(
                        ConstValue { ty: ScalarType::Byte, bits: 0 }.number_literal(),
                    )),
                    span,
                }),
            },
            span,
        }
    }

    pub(super) fn null_comparison_operands(
        &mut self,
        scope: ScopeId,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Option<(subject::SemExpr, subject::SemExpr)> {
        if !matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
            return None;
        }
        if self.is_builtin_null(scope, left) {
            let right = self.expect_expr(scope, right, right.span);
            let left = self.lower_null(scope, left.span, Some(&right.ty));
            return Some((left, right));
        }
        if self.is_builtin_null(scope, right) {
            let left = self.expect_expr(scope, left, left.span);
            let right = self.lower_null(scope, right.span, Some(&left.ty));
            return Some((left, right));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::tokenize, parser::parse};

    fn analyze_null(source: &str, target: TargetId) -> Result<(Program, SemanticModel), Vec<Diagnostic>> {
        let program = parse(&tokenize(source).unwrap()).unwrap();
        let model = analyze_with_options(&program, SemanticOptions::modern().with_target(target))?;
        Ok((program, model))
    }

    const SOURCE: &str = "TYPE Handle=[BYTE tag]\n\
        Handle POINTER file=[NULL]\nBYTE POINTER bytes\nPROC POINTER callback\n\
        BYTE result\n\
        Handle POINTER FUNC Empty() RETURN(NULL)\n\
        PROC Take(Handle POINTER value) result=value=NULL RETURN\n\
        PROC Main()\n\
        file=NULL bytes=null callback=NuLl\n\
        LET Handle POINTER saved=NULL\n\
        result=file=NULL result=NULL<>file result=saved=NULL\n\
        Take(NULL) file=Empty() bytes=BYTE POINTER(NULL)\nRETURN\n";

    #[test]
    fn null_values_acquire_pointer_types_before_nir_on_all_targets() {
        for target in [TargetId::Atari6502, TargetId::Wdc65816Small,
                       TargetId::Wdc65816Native, TargetId::Motorola68000] {
            let (ast, model) = analyze_null(SOURCE, target).unwrap();
            assert!(model.null_pointer_types.len() >= 10);
            assert!(model.null_pointer_types.values()
                .all(|ty| ty.is_pointer() || ty.as_callable_pointer().is_some()));
            let semir = ir::lower_program(&ast, &model);
            let nir = crate::nir::lower_program(&semir);
            crate::nir::verify_program(&nir).unwrap();
            let optimized = crate::nir::optimize_program(&nir).unwrap();
            crate::nir::verify_program(&optimized).unwrap();
        }
    }

    #[test]
    fn null_values_support_cstring_and_pointer_leaves_in_initializers() {
        let source = "TYPE Row=[BYTE tag BYTE POINTER data PROC POINTER callback]\n\
            Row item=[1 NULL NULL]\nBYTE POINTER ARRAY pointers(2)=[NULL NULL]\n\
            CSTRING text\nCSTRING FUNC Empty() RETURN(NULL)\n\
            PROC Main() text=NULL text=CSTRING(NULL) text=Empty() RETURN\n";
        let (ast, model) = analyze_null(source, TargetId::Wdc65816Native).unwrap();
        let semir = ir::lower_program(&ast, &model);
        crate::nir::verify_program(&crate::nir::lower_program(&semir)).unwrap();
    }

    #[test]
    fn null_values_reject_nonpointer_and_untyped_contexts() {
        for source in [
            "BYTE value PROC Main() value=NULL RETURN",
            "PROC Main() LET value=NULL RETURN",
            "CARD FUNC Empty() RETURN(NULL) PROC Main() RETURN",
            "PROC Take(CARD value) RETURN PROC Main() Take(NULL) RETURN",
            "BYTE POINTER p PROC Main() IF p<NULL THEN RETURN FI RETURN",
            "BYTE POINTER p PROC Main() p=p+NULL RETURN",
            "BYTE POINTER p PROC Main() IF NULL=NULL THEN RETURN FI RETURN",
            "PROC Main() IF NULL THEN RETURN FI RETURN",
            "PROC Main() LET value=CARD(NULL) RETURN",
            "BYTE ARRAY data(1)=[NULL] PROC Main() RETURN",
            "BYTE POINTER p=[-NULL] PROC Main() RETURN",
            "BYTE POINTER p=NULL PROC Main() RETURN",
        ] {
            let errors = analyze_null(source, TargetId::Wdc65816Native).unwrap_err();
            assert!(errors.iter().any(|d| d.message.contains("NULL")), "{source}: {errors:?}");
        }
    }

    #[test]
    fn null_values_preserve_declared_names_and_compatibility_profile() {
        for source in [
            "CONST NULL=7 BYTE value PROC Main() value=NULL RETURN",
            "BYTE null PROC Main() null=7 RETURN",
            "DEFINE NULL=\"0\" BYTE value PROC Main() value=NULL RETURN",
        ] {
            let (ast, model) = analyze_null(source, TargetId::Wdc65816Native).unwrap();
            assert!(model.null_pointer_types.is_empty());
            analyze_with_options(&ast, SemanticOptions::default()).unwrap();
        }
        let ast = parse(&tokenize("BYTE POINTER p PROC Main() p=NULL RETURN").unwrap()).unwrap();
        let errors = analyze_with_options(&ast, SemanticOptions::default()).unwrap_err();
        assert!(errors.iter().any(|d| d.message == "NULL requires the modern profile"));
    }
}
