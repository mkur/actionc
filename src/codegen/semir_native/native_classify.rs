use crate::semantic::ValueType;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeByteSource {
    Immediate(u8),
    Storage { address: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeByteRegister {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeByteSourceMode {
    Exact,
    ZeroExtendToWord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeWordSource {
    pub(super) low: NativeByteSource,
    pub(super) high: NativeByteSource,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeArrayIndexExpr<'a> {
    pub(super) symbol: &'a SemSymbolRef,
    pub(super) index: &'a SemExpr,
}

#[derive(Debug, Clone)]
pub(super) struct NativeArrayIndexAccess<'a> {
    pub(super) slot: NativeStorageSlot,
    pub(super) index: &'a SemExpr,
    pub(super) element_width: u16,
    pub(super) storage: CodegenArrayStorage,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativePointerDeref<'a> {
    pub(super) pointer: &'a SemExpr,
    pub(super) width: u16,
}

#[derive(Debug, Clone)]
pub(super) struct NativePointerIndexExpr<'a> {
    pub(super) base: NativeResolvedSlot,
    pub(super) index: &'a SemExpr,
    pub(super) element_width: u16,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeRecordFieldAccess<'a> {
    pub(super) base: &'a SemLValue,
    pub(super) field: &'a SemFieldRef,
    pub(super) width: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NativeValueShape {
    Literal { value: u16, width: u16 },
    Storage(NativeResolvedSlot),
    Address(NativeAddressShape),
    Deref { pointer: String, width: u16 },
    Indexed(NativeIndexedShape),
    CallResult { callee: String, width: Option<u16> },
    Computed { width: Option<u16> },
    Unsupported { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeAddressShape {
    pub(super) kind: NativeAddressKind,
    pub(super) source: String,
    pub(super) address: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeAddressKind {
    StorageBase,
    StoragePointer,
    StringLiteral,
    Routine,
    CurrentLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeIndexedShape {
    pub(super) base: String,
    pub(super) index: String,
    pub(super) element_width: u16,
    pub(super) storage: NativeIndexedStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeIndexedStorage {
    Inline,
    Descriptor,
    ArrayPointer,
    PointerValue,
}

pub(super) struct NativeClassifier<'e, 'a, 'm> {
    emitter: &'e SemIrNativeEmitter<'a, 'm>,
}

impl<'a, 'm> SemIrNativeEmitter<'a, 'm> {
    pub(super) fn classifier(&self) -> NativeClassifier<'_, 'a, 'm> {
        NativeClassifier { emitter: self }
    }
}

impl<'e, 'a, 'm> NativeClassifier<'e, 'a, 'm> {
    pub(super) fn value_shape(&self, expr: &SemExpr) -> Result<NativeValueShape, String> {
        self.emitter.classify_value(expr)
    }

    #[cfg(test)]
    pub(super) fn lvalue_shape(&self, lvalue: &SemLValue) -> Result<NativeValueShape, String> {
        self.emitter.classify_lvalue_value(lvalue)
    }

    pub(super) fn address_shape(
        &self,
        expr: &SemExpr,
    ) -> Result<Option<NativeAddressShape>, String> {
        match self.value_shape(expr)? {
            NativeValueShape::Address(address) => Ok(Some(address)),
            _ => Ok(None),
        }
    }

    pub(super) fn byte_source(
        &self,
        expr: &SemExpr,
        byte_index: u16,
        mode: NativeByteSourceMode,
    ) -> Result<Option<NativeByteSource>, String> {
        self.emitter.classified_byte_source(expr, byte_index, mode)
    }

    pub(super) fn value_byte_source(
        &self,
        expr: &SemExpr,
        byte_index: u16,
    ) -> Result<Option<NativeByteSource>, String> {
        self.byte_source(expr, byte_index, NativeByteSourceMode::Exact)
    }

    pub(super) fn byte_operand_source(
        &self,
        expr: &SemExpr,
    ) -> Result<Option<NativeByteSource>, String> {
        if let Some(value) = literal_byte(expr) {
            return Ok(Some(NativeByteSource::Immediate(value)));
        }
        let Some(slot) = self.addressable_slot(expr)? else {
            return Ok(None);
        };
        if !matches!(slot.width, 1 | 2) {
            return Err("byte operand source width mismatch".to_string());
        }
        Ok(Some(NativeByteSource::Storage {
            address: slot.address,
        }))
    }

    pub(super) fn compare_byte_source(
        &self,
        expr: &SemExpr,
        byte_index: u16,
    ) -> Result<Option<NativeByteSource>, String> {
        self.byte_source(expr, byte_index, NativeByteSourceMode::ZeroExtendToWord)
    }

    pub(super) fn word_source(
        &self,
        expr: &SemExpr,
        mode: NativeByteSourceMode,
    ) -> Result<Option<NativeWordSource>, String> {
        let Some(low) = self.byte_source(expr, 0, mode)? else {
            return Ok(None);
        };
        let Some(high) = self.byte_source(expr, 1, mode)? else {
            return Ok(None);
        };
        Ok(Some(NativeWordSource { low, high }))
    }

    pub(super) fn value_width(&self, expr: &SemExpr) -> Result<u16, String> {
        match self.value_shape(expr)? {
            NativeValueShape::Literal { width, .. } => Ok(width),
            NativeValueShape::Storage(slot) => Ok(slot.width),
            NativeValueShape::Address(_) => Ok(2),
            NativeValueShape::Deref { width, .. } => Ok(width),
            NativeValueShape::Indexed(indexed) => Ok(indexed.element_width),
            NativeValueShape::CallResult {
                width: Some(width), ..
            }
            | NativeValueShape::Computed { width: Some(width) } => Ok(width),
            NativeValueShape::CallResult {
                width: None,
                callee,
            } => Err(format!("call result `{callee}` has no value width")),
            NativeValueShape::Computed { width: None } => Err(format!(
                "value width is not known: {}",
                native_expr_debug_name(expr)
            )),
            NativeValueShape::Unsupported { reason } => {
                Err(format!("unsupported value shape: {reason}"))
            }
        }
    }

    pub(super) fn addressable_slot(
        &self,
        expr: &SemExpr,
    ) -> Result<Option<NativeResolvedSlot>, String> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.addressable_slot(expr),
            SemExprKind::Symbol(symbol) => self.emitter.resolved_symbol_slot(symbol).map(Some),
            SemExprKind::LValue(lvalue) => self.addressable_lvalue_slot(lvalue),
            SemExprKind::ArrayDecay(decay) if decay.origin == SemArrayOrigin::Parameter => {
                self.emitter.resolved_lvalue_slot(&decay.array).map(Some)
            }
            SemExprKind::Call(call) => {
                let SemCallable::User(symbol) = &call.callee else {
                    return Ok(None);
                };
                let Some(slot) = self.emitter.storage.get(&symbol.id) else {
                    return Ok(None);
                };
                if slot.array.is_none()
                    || call.args.len() != 1
                    || literal_word(&call.args[0]).is_none()
                {
                    return Ok(None);
                }
                self.emitter.resolved_call_index_slot(call).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn required_addressable_slot(
        &self,
        expr: &SemExpr,
    ) -> Result<NativeResolvedSlot, String> {
        self.addressable_slot(expr)?.ok_or_else(|| {
            "only symbol and constant-index assignment sources are supported".to_string()
        })
    }

    pub(super) fn required_lvalue_slot(
        &self,
        lvalue: &SemLValue,
    ) -> Result<NativeResolvedSlot, String> {
        self.addressable_lvalue_slot(lvalue)?.ok_or_else(|| {
            "only symbol and constant-index assignment targets are supported".to_string()
        })
    }

    pub(super) fn pointer_base_slot(
        &self,
        expr: &SemExpr,
    ) -> Result<Option<NativeResolvedSlot>, String> {
        let symbol = match &expr.kind {
            SemExprKind::Symbol(symbol) => symbol,
            SemExprKind::LValue(lvalue) => match &lvalue.kind {
                SemLValueKind::Symbol(symbol) => symbol,
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let Some(slot) = self.emitter.storage.get(&symbol.id).cloned() else {
            return Ok(None);
        };
        if slot.pointee_width.is_none() {
            return Ok(None);
        }
        Ok(Some(NativeResolvedSlot {
            address: slot.address,
            width: slot.width,
            pointee_width: slot.pointee_width,
            record: slot.record.clone(),
        }))
    }

    pub(super) fn is_array_index_call(&self, call: &SemCall) -> bool {
        let SemCallable::User(symbol) = &call.callee else {
            return false;
        };
        self.emitter
            .storage
            .get(&symbol.id)
            .is_some_and(|slot| slot.array.is_some())
    }

    pub(super) fn is_pointer_index_call(&self, call: &SemCall) -> bool {
        let SemCallable::User(symbol) = &call.callee else {
            return false;
        };
        self.emitter
            .storage
            .get(&symbol.id)
            .is_some_and(|slot| slot.pointee_width.is_some())
    }

    pub(super) fn routine_call_expr<'s>(&self, expr: &'s SemExpr) -> Option<&'s SemCall> {
        let SemExprKind::Call(call) = &expr.kind else {
            return None;
        };
        (!self.is_array_index_call(call) && !self.is_pointer_index_call(call)).then_some(call)
    }

    pub(super) fn pointer_deref_expr<'s>(
        &self,
        expr: &'s SemExpr,
    ) -> Option<NativePointerDeref<'s>> {
        let SemExprKind::LValue(lvalue) = &expr.kind else {
            return None;
        };
        self.pointer_deref_lvalue(lvalue)
    }

    pub(super) fn pointer_deref_lvalue<'s>(
        &self,
        lvalue: &'s SemLValue,
    ) -> Option<NativePointerDeref<'s>> {
        let SemLValueKind::Deref { pointer } = &lvalue.kind else {
            return None;
        };
        Some(NativePointerDeref {
            pointer,
            width: lvalue.type_facts().width.unwrap_or(1),
        })
    }

    pub(super) fn pointer_index_expr<'s>(
        &self,
        expr: &'s SemExpr,
    ) -> Result<Option<NativePointerIndexExpr<'s>>, String> {
        match &expr.kind {
            SemExprKind::Call(call) => self.pointer_index_call(call),
            SemExprKind::LValue(lvalue) => self.pointer_index_lvalue(lvalue),
            _ => Ok(None),
        }
    }

    pub(super) fn pointer_index_lvalue<'s>(
        &self,
        lvalue: &'s SemLValue,
    ) -> Result<Option<NativePointerIndexExpr<'s>>, String> {
        let SemLValueKind::Index { base, index, .. } = &lvalue.kind else {
            return Ok(None);
        };
        let Some(base) = self.pointer_base_slot(base)? else {
            return Ok(None);
        };
        let Some(element_width) = base.pointee_width else {
            return Ok(None);
        };
        Ok(Some(NativePointerIndexExpr {
            base,
            index: index.as_ref(),
            element_width,
        }))
    }

    fn pointer_index_call<'s>(
        &self,
        call: &'s SemCall,
    ) -> Result<Option<NativePointerIndexExpr<'s>>, String> {
        let SemCallable::User(symbol) = &call.callee else {
            return Ok(None);
        };
        if call.args.len() != 1 {
            return Ok(None);
        }
        let Some(slot) = self.emitter.storage.get(&symbol.id).cloned() else {
            return Ok(None);
        };
        let Some(element_width) = slot.pointee_width else {
            return Ok(None);
        };
        Ok(Some(NativePointerIndexExpr {
            base: NativeResolvedSlot {
                address: slot.address,
                width: slot.width,
                pointee_width: slot.pointee_width,
                record: slot.record,
            },
            index: &call.args[0],
            element_width,
        }))
    }

    pub(super) fn record_field_expr<'s>(
        &self,
        expr: &'s SemExpr,
    ) -> Option<NativeRecordFieldAccess<'s>> {
        let SemExprKind::LValue(lvalue) = &expr.kind else {
            return None;
        };
        self.record_field_lvalue(lvalue)
    }

    pub(super) fn record_field_lvalue<'s>(
        &self,
        lvalue: &'s SemLValue,
    ) -> Option<NativeRecordFieldAccess<'s>> {
        let SemLValueKind::Field { base, field } = &lvalue.kind else {
            return None;
        };
        Some(NativeRecordFieldAccess {
            base,
            field,
            width: lvalue.type_facts().width.unwrap_or(1),
        })
    }

    pub(super) fn array_index_expr<'s>(
        &self,
        expr: &'s SemExpr,
    ) -> Option<NativeArrayIndexExpr<'s>> {
        match &expr.kind {
            SemExprKind::Call(call) => {
                let SemCallable::User(symbol) = &call.callee else {
                    return None;
                };
                if call.args.len() == 1 && self.is_array_index_call(call) {
                    Some(NativeArrayIndexExpr {
                        symbol,
                        index: &call.args[0],
                    })
                } else {
                    None
                }
            }
            SemExprKind::LValue(lvalue) => {
                let SemLValueKind::Index { base, index, .. } = &lvalue.kind else {
                    return None;
                };
                let symbol = self.array_base_symbol(base)?;
                Some(NativeArrayIndexExpr {
                    symbol,
                    index: index.as_ref(),
                })
            }
            _ => None,
        }
    }

    pub(super) fn array_index_access<'s>(
        &self,
        expr: &'s SemExpr,
    ) -> Result<Option<NativeArrayIndexAccess<'s>>, String> {
        let Some(indexed) = self.array_index_expr(expr) else {
            return Ok(None);
        };
        self.array_index_access_from_parts(indexed.symbol, indexed.index)
    }

    pub(super) fn array_index_lvalue<'s>(
        &self,
        lvalue: &'s SemLValue,
    ) -> Result<Option<NativeArrayIndexAccess<'s>>, String> {
        let SemLValueKind::Index { base, index, .. } = &lvalue.kind else {
            return Ok(None);
        };
        let Some(symbol) = self.array_base_symbol(base) else {
            return Ok(None);
        };
        self.array_index_access_from_parts(symbol, index.as_ref())
    }

    fn array_index_access_from_parts<'s>(
        &self,
        symbol: &'s SemSymbolRef,
        index: &'s SemExpr,
    ) -> Result<Option<NativeArrayIndexAccess<'s>>, String> {
        let slot = self
            .emitter
            .storage
            .get(&symbol.id)
            .cloned()
            .or_else(|| native_builtin_array_storage_slot(&symbol.name))
            .ok_or_else(|| format!("symbol `{}` has no native storage", symbol.name))?;
        let Some(array) = slot.array else {
            return Ok(None);
        };
        Ok(Some(NativeArrayIndexAccess {
            slot,
            index,
            element_width: array.element_width,
            storage: array.storage,
        }))
    }

    pub(super) fn array_base_symbol<'s>(&self, expr: &'s SemExpr) -> Option<&'s SemSymbolRef> {
        match &expr.kind {
            SemExprKind::Symbol(symbol) => Some(symbol),
            SemExprKind::LValue(lvalue)
            | SemExprKind::ArrayDecay(SemArrayDecay { array: lvalue, .. }) => {
                self.array_base_lvalue_symbol(lvalue)
            }
            _ => None,
        }
    }

    fn array_base_lvalue_symbol<'s>(&self, lvalue: &'s SemLValue) -> Option<&'s SemSymbolRef> {
        match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => Some(symbol),
            _ => None,
        }
    }

    fn addressable_lvalue_slot(
        &self,
        lvalue: &SemLValue,
    ) -> Result<Option<NativeResolvedSlot>, String> {
        match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => self.emitter.resolved_symbol_slot(symbol).map(Some),
            SemLValueKind::Index { index, .. } if literal_word(index).is_some() => {
                self.emitter.resolved_lvalue_slot(lvalue).map(Some)
            }
            SemLValueKind::Field { .. } => match self.emitter.resolved_lvalue_slot(lvalue) {
                Ok(slot) => Ok(Some(slot)),
                Err(_) => Ok(None),
            },
            SemLValueKind::Index { .. } | SemLValueKind::Deref { .. } => Ok(None),
            SemLValueKind::UnresolvedName(name) => {
                Err(format!("unresolved lvalue `{name}` has no native storage"))
            }
        }
    }
}

impl<'a, 'm> SemIrNativeEmitter<'a, 'm> {
    pub(super) fn classify_value(&self, expr: &SemExpr) -> Result<NativeValueShape, String> {
        match &expr.kind {
            SemExprKind::Cast { expr, .. } => self.classify_value(expr),
            SemExprKind::Literal(SemLiteral::String(text)) => {
                Ok(NativeValueShape::Address(NativeAddressShape {
                    kind: NativeAddressKind::StringLiteral,
                    source: format!("\"{text}\""),
                    address: None,
                }))
            }
            SemExprKind::Literal(_) => {
                let value = literal_word(expr).ok_or_else(|| {
                    format!(
                        "literal `{}` has no native value",
                        native_expr_debug_name(expr)
                    )
                })?;
                Ok(NativeValueShape::Literal {
                    value,
                    width: expr_width(expr).unwrap_or(2),
                })
            }
            SemExprKind::CurrentLocation => Ok(NativeValueShape::Address(NativeAddressShape {
                kind: NativeAddressKind::CurrentLocation,
                source: "*".to_string(),
                address: Some(self.current_address()?),
            })),
            SemExprKind::Symbol(symbol) => self.classify_symbol_value(symbol),
            SemExprKind::LValue(lvalue) => self.classify_lvalue_value(lvalue),
            SemExprKind::AddressOf(lvalue) => self.classify_lvalue_address(lvalue),
            SemExprKind::ImplicitAddressOf(address) => self.classify_lvalue_address(&address.place),
            SemExprKind::AddressOfSymbol(symbol) => self.classify_symbol_address(symbol),
            SemExprKind::ArrayDecay(decay) => self.classify_array_decay(decay),
            SemExprKind::Call(call) => self.classify_call_value(call),
            SemExprKind::Unary { .. } | SemExprKind::Binary { .. } => {
                Ok(NativeValueShape::Computed {
                    width: expr_width(expr),
                })
            }
            SemExprKind::Missing => Ok(NativeValueShape::Unsupported {
                reason: "missing expression".to_string(),
            }),
            SemExprKind::Raw(raw) => Ok(NativeValueShape::Unsupported {
                reason: format!("raw expression `{raw}`"),
            }),
            SemExprKind::UnresolvedName(name) => Ok(NativeValueShape::Unsupported {
                reason: format!("unresolved expression `{name}`"),
            }),
        }
    }

    fn classify_symbol_value(&self, symbol: &SemSymbolRef) -> Result<NativeValueShape, String> {
        if self.routine_labels.contains_key(&symbol.id)
            || self.routine_entries.contains_key(&symbol.id)
        {
            return self.classify_symbol_address(symbol);
        }

        if let Some(value) = self.numeric_define(&symbol.name) {
            return Ok(NativeValueShape::Literal {
                value,
                width: symbol
                    .ty
                    .as_ref()
                    .and_then(ValueType::value_width_bytes)
                    .unwrap_or_else(|| native_literal_width(value)),
            });
        }

        let Some(slot) = self.storage.get(&symbol.id).cloned() else {
            if let Some(slot) = native_builtin_array_storage_slot(&symbol.name)
                && let Some(array) = slot.array
            {
                return Ok(NativeValueShape::Address(NativeAddressShape {
                    kind: native_address_kind_for_array(array.storage),
                    source: symbol.name.clone(),
                    address: Some(slot.address),
                }));
            }
            if let Some(slot) = native_builtin_variable_slot(&symbol.name) {
                return Ok(NativeValueShape::Storage(slot));
            }
            return Err(format!("symbol `{}` has no native storage", symbol.name));
        };
        if let Some(array) = slot.array
            && !slot_is_array_pointer_value(&slot)
        {
            return Ok(NativeValueShape::Address(NativeAddressShape {
                kind: native_address_kind_for_array(array.storage),
                source: symbol.name.clone(),
                address: Some(slot.address),
            }));
        }
        Ok(NativeValueShape::Storage(NativeResolvedSlot {
            address: slot.address,
            width: slot.width,
            pointee_width: slot.pointee_width,
            record: slot.record,
        }))
    }

    pub(super) fn classify_lvalue_value(
        &self,
        lvalue: &SemLValue,
    ) -> Result<NativeValueShape, String> {
        match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => self.classify_symbol_value(symbol),
            SemLValueKind::Deref { pointer } => Ok(NativeValueShape::Deref {
                pointer: native_expr_debug_name(pointer),
                width: lvalue.type_facts().width.unwrap_or(1),
            }),
            SemLValueKind::Index { base, index, .. } => self.classify_indexed_value(base, index),
            SemLValueKind::Field { .. } => {
                if let Ok(slot) = self.resolved_lvalue_slot(lvalue) {
                    return Ok(NativeValueShape::Storage(slot));
                }
                Ok(NativeValueShape::Deref {
                    pointer: native_lvalue_debug_name(lvalue),
                    width: lvalue.type_facts().width.unwrap_or(1),
                })
            }
            SemLValueKind::UnresolvedName(name) => Ok(NativeValueShape::Unsupported {
                reason: format!("unresolved lvalue `{name}`"),
            }),
        }
    }

    fn classify_lvalue_address(&self, lvalue: &SemLValue) -> Result<NativeValueShape, String> {
        if let Some(symbol) = self.lvalue_symbol(lvalue) {
            return self.classify_symbol_address(symbol);
        }
        let address = self.lvalue_address(lvalue)?;
        Ok(NativeValueShape::Address(NativeAddressShape {
            kind: NativeAddressKind::StorageBase,
            source: native_lvalue_debug_name(lvalue),
            address: Some(address),
        }))
    }

    fn classify_symbol_address(&self, symbol: &SemSymbolRef) -> Result<NativeValueShape, String> {
        if let Some(slot) = self.storage.get(&symbol.id) {
            return Ok(NativeValueShape::Address(NativeAddressShape {
                kind: slot
                    .array
                    .map(|array| native_address_kind_for_array(array.storage))
                    .unwrap_or(NativeAddressKind::StorageBase),
                source: symbol.name.clone(),
                address: Some(slot.address),
            }));
        }
        if let Some(slot) = native_builtin_variable_slot(&symbol.name) {
            return Ok(NativeValueShape::Address(NativeAddressShape {
                kind: NativeAddressKind::StorageBase,
                source: symbol.name.clone(),
                address: Some(slot.address),
            }));
        }
        if self.routine_labels.contains_key(&symbol.id)
            || self.routine_entries.contains_key(&symbol.id)
        {
            return Ok(NativeValueShape::Address(NativeAddressShape {
                kind: NativeAddressKind::Routine,
                source: symbol.name.clone(),
                address: self.routine_entries.get(&symbol.id).copied(),
            }));
        }
        Err(format!(
            "symbol `{}` has no native addressable storage",
            symbol.name
        ))
    }

    fn classify_array_decay(&self, decay: &SemArrayDecay) -> Result<NativeValueShape, String> {
        let source = native_lvalue_debug_name(&decay.array);
        if decay.origin == SemArrayOrigin::Parameter {
            let slot = self.resolved_lvalue_slot(&decay.array)?;
            return Ok(NativeValueShape::Address(NativeAddressShape {
                kind: NativeAddressKind::StoragePointer,
                source,
                address: Some(slot.address),
            }));
        }
        let symbol = self
            .lvalue_symbol(&decay.array)
            .ok_or_else(|| format!("array decay `{source}` has no native symbol"))?;
        let slot = self
            .storage
            .get(&symbol.id)
            .ok_or_else(|| format!("array decay `{source}` has no native storage"))?;
        Ok(NativeValueShape::Address(NativeAddressShape {
            kind: slot
                .array
                .map(|array| native_address_kind_for_array(array.storage))
                .unwrap_or(NativeAddressKind::StorageBase),
            source,
            address: Some(slot.address),
        }))
    }

    fn classify_call_value(&self, call: &SemCall) -> Result<NativeValueShape, String> {
        if self.classifier().is_array_index_call(call) {
            let SemCallable::User(symbol) = &call.callee else {
                unreachable!("array index classification already checked callable");
            };
            if call.args.len() != 1 {
                return Ok(NativeValueShape::Unsupported {
                    reason: format!("array `{}` needs exactly one index", symbol.name),
                });
            }
            let slot = self
                .storage
                .get(&symbol.id)
                .cloned()
                .ok_or_else(|| format!("symbol `{}` has no native storage", symbol.name))?;
            let Some(array) = slot.array else {
                return Ok(NativeValueShape::Unsupported {
                    reason: format!("symbol `{}` is not an array", symbol.name),
                });
            };
            return Ok(NativeValueShape::Indexed(NativeIndexedShape {
                base: symbol.name.clone(),
                index: native_expr_debug_name(&call.args[0]),
                element_width: array.element_width,
                storage: native_indexed_storage_for_array(array.storage),
            }));
        }

        if self.classifier().is_pointer_index_call(call) {
            let SemCallable::User(symbol) = &call.callee else {
                unreachable!("pointer index classification already checked callable");
            };
            if call.args.len() != 1 {
                return Ok(NativeValueShape::Unsupported {
                    reason: format!("pointer `{}` needs exactly one index", symbol.name),
                });
            }
            let slot = self
                .storage
                .get(&symbol.id)
                .ok_or_else(|| format!("symbol `{}` has no native storage", symbol.name))?;
            return Ok(NativeValueShape::Indexed(NativeIndexedShape {
                base: symbol.name.clone(),
                index: native_expr_debug_name(&call.args[0]),
                element_width: slot.pointee_width.unwrap_or(1),
                storage: NativeIndexedStorage::PointerValue,
            }));
        }

        Ok(NativeValueShape::CallResult {
            callee: native_call_debug_name(call),
            width: call
                .return_type
                .as_ref()
                .and_then(ValueType::value_width_bytes),
        })
    }

    fn classify_indexed_value(
        &self,
        base: &SemExpr,
        index: &SemExpr,
    ) -> Result<NativeValueShape, String> {
        if let Some(pointer) = self.classifier().pointer_base_slot(base)? {
            return Ok(NativeValueShape::Indexed(NativeIndexedShape {
                base: native_expr_debug_name(base),
                index: native_expr_debug_name(index),
                element_width: pointer.pointee_width.unwrap_or(1),
                storage: NativeIndexedStorage::PointerValue,
            }));
        }

        let (name, slot) = self.array_slot_from_expr(base)?;
        let Some(array) = slot.array else {
            return Ok(NativeValueShape::Unsupported {
                reason: format!("symbol `{name}` is not an array"),
            });
        };
        Ok(NativeValueShape::Indexed(NativeIndexedShape {
            base: name,
            index: native_expr_debug_name(index),
            element_width: array.element_width,
            storage: native_indexed_storage_for_array(array.storage),
        }))
    }

    pub(super) fn lvalue_base_address(&self, lvalue: &SemLValue) -> Option<u16> {
        let symbol = self.lvalue_symbol(lvalue)?;
        self.storage
            .get(&symbol.id)
            .map(|slot| slot.address)
            .or_else(|| native_builtin_variable_slot(&symbol.name).map(|slot| slot.address))
    }

    pub(super) fn lvalue_address(&self, lvalue: &SemLValue) -> Result<u16, String> {
        if let Ok(slot) = self.resolved_lvalue_slot(lvalue) {
            return Ok(slot.address);
        }
        self.lvalue_base_address(lvalue).ok_or_else(|| {
            format!(
                "lvalue `{}` has no native address",
                native_lvalue_debug_name(lvalue)
            )
        })
    }

    fn classified_byte_source(
        &self,
        expr: &SemExpr,
        byte_index: u16,
        mode: NativeByteSourceMode,
    ) -> Result<Option<NativeByteSource>, String> {
        if let Some(source) = self.shifted_byte_source(expr, byte_index, mode)? {
            return Ok(Some(source));
        }
        match self.classify_value(expr)? {
            NativeValueShape::Literal { value, .. } => self.literal_byte_source(value, byte_index),
            NativeValueShape::Storage(slot) => {
                self.classified_storage_byte_source(slot.address, slot.width, byte_index, mode)
            }
            NativeValueShape::Address(NativeAddressShape {
                kind:
                    NativeAddressKind::StorageBase
                    | NativeAddressKind::Routine
                    | NativeAddressKind::CurrentLocation,
                address: Some(address),
                ..
            }) => {
                if byte_index >= 2 {
                    return Err("call argument address byte index is out of bounds".to_string());
                }
                Ok(Some(NativeByteSource::Immediate(if byte_index == 0 {
                    (address & 0x00FF) as u8
                } else {
                    (address >> 8) as u8
                })))
            }
            NativeValueShape::Address(NativeAddressShape {
                kind: NativeAddressKind::StoragePointer,
                address: Some(address),
                ..
            }) => self.classified_storage_byte_source(address, 2, byte_index, mode),
            NativeValueShape::Address(NativeAddressShape {
                kind: NativeAddressKind::StringLiteral,
                ..
            })
            | NativeValueShape::Address(NativeAddressShape { address: None, .. })
            | NativeValueShape::Deref { .. }
            | NativeValueShape::Indexed(_)
            | NativeValueShape::CallResult { .. }
            | NativeValueShape::Computed { .. }
            | NativeValueShape::Unsupported { .. } => Ok(None),
        }
    }

    fn shifted_byte_source(
        &self,
        expr: &SemExpr,
        byte_index: u16,
        mode: NativeByteSourceMode,
    ) -> Result<Option<NativeByteSource>, String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.shifted_byte_source(expr, byte_index, mode);
        }
        let SemExprKind::Binary {
            op: BinaryOp::Rsh,
            left,
            right,
        } = &expr.kind
        else {
            return Ok(None);
        };
        let Some(count) = literal_word(right) else {
            return Ok(None);
        };
        if count % 8 != 0 {
            return Ok(None);
        }
        let Some(source_index) = byte_index.checked_add(count / 8) else {
            return Ok(Some(NativeByteSource::Immediate(0)));
        };
        if matches!(self.classifier().value_width(left), Ok(width) if source_index >= width) {
            return Ok(Some(NativeByteSource::Immediate(0)));
        }
        self.classified_byte_source(left, source_index, mode)
    }

    fn literal_byte_source(
        &self,
        value: u16,
        byte_index: u16,
    ) -> Result<Option<NativeByteSource>, String> {
        if byte_index >= 2 {
            return Err("literal byte index is out of bounds".to_string());
        }
        Ok(Some(NativeByteSource::Immediate(
            ((value >> (8 * byte_index)) & 0x00FF) as u8,
        )))
    }

    fn classified_storage_byte_source(
        &self,
        address: u16,
        width: u16,
        byte_index: u16,
        mode: NativeByteSourceMode,
    ) -> Result<Option<NativeByteSource>, String> {
        if byte_index >= width {
            if mode == NativeByteSourceMode::ZeroExtendToWord && byte_index < 2 {
                return Ok(Some(NativeByteSource::Immediate(0)));
            }
            return Err("byte source index is out of bounds".to_string());
        }
        Ok(Some(NativeByteSource::Storage {
            address: address
                .checked_add(byte_index)
                .ok_or_else(|| "byte source address overflow".to_string())?,
        }))
    }
}
