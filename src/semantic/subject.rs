use crate::ast::{BinaryOp, UnaryOp};
use crate::lexer::NumberLiteral;
use crate::source::Span;

use super::{CallableType, FieldId, SymbolId, ValueType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemSubject {
    Expr(SemExpr),
    Place(SemPlace),
    Callable(SemCallable),
    TypeRef(SemTypeRef),
    Define(SemDefineRef),
    Error(SemErrorSubject),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemExpr {
    pub ty: ValueType,
    pub kind: SemExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemExprKind {
    Literal(SemLiteral),
    CurrentLocation,
    Load(Box<SemPlace>),
    AddressOf(Box<SemPlace>),
    AddressOfSymbol(SymbolId),
    Cast {
        ty: ValueType,
        expr: Box<SemExpr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<SemExpr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<SemExpr>,
        right: Box<SemExpr>,
    },
    Call {
        callee: Box<SemCallable>,
        args: Vec<SemExpr>,
    },
    Raw(String),
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemLiteral {
    Number(NumberLiteral),
    String(String),
    Char(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemPlace {
    pub ty: ValueType,
    pub access: PlaceAccess,
    pub kind: SemPlaceKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemPlaceKind {
    Symbol(SymbolId),
    Field {
        base: Box<SemPlace>,
        field: SemFieldRef,
    },
    Index {
        base: Box<SemPlace>,
        index: Box<SemExpr>,
    },
    Deref(Box<SemExpr>),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceAccess {
    Assignable,
    RoutineTargetOnly,
    ReadOnly,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemFieldRef {
    pub id: Option<FieldId>,
    pub owner: Option<SymbolId>,
    pub name: String,
    pub ty: ValueType,
    pub offset: Option<u16>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemCallable {
    pub ty: CallableType,
    pub kind: SemCallableKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemCallableKind {
    Function(SymbolId),
    Builtin(SymbolId),
    FunctionValue(Box<SemExpr>),
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemTypeRef {
    pub ty: ValueType,
    pub kind: SemTypeRefKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemTypeRefKind {
    Symbol(SymbolId),
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemDefineRef {
    pub symbol: SymbolId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemErrorSubject {
    pub span: Span,
}
