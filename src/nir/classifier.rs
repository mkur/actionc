use crate::ast::{BinaryOp, UnaryOp};
use crate::semantic::{
    SymbolClass,
    ir::{SemCall, SemCallable},
};

use super::ir::{NirBinaryOp, NirCompareOp, NirUnaryOp};

pub(super) struct NirClassifier;

impl NirClassifier {
    pub(super) fn is_materializable_call(call: &SemCall) -> bool {
        match &call.callee {
            SemCallable::User(symbol) => {
                matches!(symbol.class, SymbolClass::Proc | SymbolClass::Func)
            }
            SemCallable::Builtin(symbol) => {
                matches!(
                    symbol.class,
                    SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc
                )
            }
            SemCallable::Indirect { .. } | SemCallable::Runtime { .. } => true,
        }
    }

    pub(super) fn is_index_call_syntax(call: &SemCall) -> bool {
        matches!(
            &call.callee,
            SemCallable::User(symbol)
                if !matches!(symbol.class, SymbolClass::Proc | SymbolClass::Func)
        ) && call.args.len() == 1
    }

    pub(super) fn is_nir_compare_op(op: BinaryOp) -> bool {
        Self::compare_op(op).is_some()
    }

    pub(super) fn unary_op(op: UnaryOp) -> Option<NirUnaryOp> {
        match op {
            UnaryOp::Plus => Some(NirUnaryOp::Plus),
            UnaryOp::Neg => Some(NirUnaryOp::Neg),
            UnaryOp::AddressOf | UnaryOp::Deref => None,
        }
    }

    pub(super) fn binary_op(op: BinaryOp) -> Option<NirBinaryOp> {
        match op {
            BinaryOp::Add => Some(NirBinaryOp::Add),
            BinaryOp::Sub => Some(NirBinaryOp::Sub),
            BinaryOp::Mul => Some(NirBinaryOp::Mul),
            BinaryOp::Div => Some(NirBinaryOp::Div),
            BinaryOp::Mod => Some(NirBinaryOp::Mod),
            BinaryOp::Lsh => Some(NirBinaryOp::Lsh),
            BinaryOp::Rsh => Some(NirBinaryOp::Rsh),
            BinaryOp::And => Some(NirBinaryOp::And),
            BinaryOp::Or => Some(NirBinaryOp::Or),
            BinaryOp::Xor => Some(NirBinaryOp::Xor),
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => None,
        }
    }

    pub(super) fn compare_op(op: BinaryOp) -> Option<NirCompareOp> {
        match op {
            BinaryOp::Eq => Some(NirCompareOp::Eq),
            BinaryOp::Ne => Some(NirCompareOp::Ne),
            BinaryOp::Lt => Some(NirCompareOp::Lt),
            BinaryOp::Le => Some(NirCompareOp::Le),
            BinaryOp::Gt => Some(NirCompareOp::Gt),
            BinaryOp::Ge => Some(NirCompareOp::Ge),
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Lsh
            | BinaryOp::Rsh
            | BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Xor => None,
        }
    }
}
