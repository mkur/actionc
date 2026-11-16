//! Resolve DATA initializers before NIR: no names, raw code or expression text
//! survives in the typed initializer. Layout retains symbolic relocations.
use super::ir::{SemDataAddressTarget as Target, SemDataElement as Element, SemDataInitializer};
use super::*;

impl Analyzer {
    pub(super) fn analyze_data(&mut self, scope: ScopeId, data: &DataDecl) {
        let Some(owner) = self.symbols.lookup_exact(scope, &data.name) else {
            return;
        };
        let DataInitializer::Literal(items) = &data.initializer;
        let mut result = Vec::new();
        let mut offset = 0i32;
        for item in items {
            match self.data_element(scope, owner, offset, item, data.span) {
                Ok(element) => {
                    let bytes = match &element {
                        Element::Bytes(bytes) => bytes.len(),
                        Element::Integer { width, .. } | Element::Address { width, .. } => {
                            usize::from(*width)
                        }
                    };
                    let Some(next) = i32::try_from(bytes)
                        .ok()
                        .and_then(|n| offset.checked_add(n))
                    else {
                        self.diagnostics
                            .push(Diagnostic::new(data.span, "DATA block is too large"));
                        return;
                    };
                    let start = offset;
                    offset = next;
                    result.push(element);
                    if matches!(item, MachineItem::StringLiteral(_)) {
                        let width = self.data_address_width(owner, None);
                        result.push(Element::Address {
                            target: Target::Symbol(owner),
                            selector: None,
                            addend: start,
                            width,
                        });
                        let Some(next) = offset.checked_add(i32::from(width)) else {
                            self.diagnostics
                                .push(Diagnostic::new(data.span, "DATA block is too large"));
                            return;
                        };
                        offset = next;
                    }
                }
                Err(reason) => self.diagnostics.push(Diagnostic::new(data.span, reason)),
            }
        }
        let layout = TargetLayout::for_target(self.options.target);
        if u64::from(offset as u32) > ((1u64 << layout.size_integer_bits) - 1) {
            self.diagnostics.push(Diagnostic::new(
                data.span,
                "DATA block exceeds the target object size limit",
            ));
        }
        self.data_initializers
            .insert(owner, SemDataInitializer::Literal(result));
    }

    fn data_element(
        &mut self,
        scope: ScopeId,
        owner: SymbolId,
        offset: i32,
        item: &MachineItem,
        span: Span,
    ) -> Result<Element, String> {
        let literal = |value: u64| {
            if value > 65535 {
                return Err("DATA raw integer must fit a byte or word".into());
            }
            Ok(Element::Integer {
                value,
                width: if value <= 255 { 1 } else { 2 },
            })
        };
        let explicit_address =
            matches!(item, MachineItem::AddressExpr(expr) if expr.explicit_address);
        let pointer_width = TargetLayout::for_target(self.options.target)
            .data_pointer
            .size_bytes
            .get() as u8;
        let literal_address = |value| Element::Address {
            target: Target::Absolute(crate::target::AddressValue::data(value)),
            selector: None,
            addend: 0,
            width: pointer_width,
        };
        let (atom, selector, addend) = match item {
            MachineItem::Number(number) => {
                return literal(number.value.ok_or("DATA requires an integer literal")?);
            }
            MachineItem::StringLiteral(text) => {
                let bytes: Option<Vec<_>> = text.chars().map(source_char_byte).collect();
                let bytes =
                    bytes.ok_or("DATA string contains a character outside the byte encoding")?;
                let len = u8::try_from(bytes.len()).map_err(|_| "DATA string exceeds 255 bytes")?;
                return Ok(Element::Bytes(std::iter::once(len).chain(bytes).collect()));
            }
            MachineItem::CharLiteral(ch) => {
                return Ok(Element::Bytes(vec![
                    source_char_byte(*ch).ok_or("DATA character is outside the byte encoding")?,
                    0x9A,
                ]));
            }
            MachineItem::Name(name) => (MachineAddressAtom::Name(name.clone()), None, 0),
            MachineItem::AddressByte { name, selector } => {
                (MachineAddressAtom::Name(name.clone()), Some(*selector), 0)
            }
            MachineItem::AddressExpr(expr) => (expr.atom.clone(), expr.selector, expr.offset),
            MachineItem::Raw(_) => return Err("unsupported DATA initializer item".into()),
        };
        let target = match atom {
            MachineAddressAtom::Number(number) => {
                let value = number.value.ok_or("DATA requires an integer address")?;
                let value = value
                    .checked_add_signed(i64::from(addend))
                    .filter(|v| *v <= 65535)
                    .ok_or("DATA numeric address is outside the byte/word range")?;
                return match selector {
                    Some(AddressByteSelector::Low) => Ok(Element::Bytes(vec![value as u8])),
                    Some(AddressByteSelector::High) => Ok(Element::Bytes(vec![(value >> 8) as u8])),
                    None if explicit_address => Ok(literal_address(value)),
                    None => literal(value),
                };
            }
            MachineAddressAtom::Current => {
                return Ok(Element::Address {
                    target: Target::Symbol(owner),
                    selector,
                    addend: offset
                        .checked_add(addend)
                        .ok_or("DATA address offset overflow")?,
                    width: self.data_address_width(owner, selector),
                });
            }
            MachineAddressAtom::Name(name) => self
                .resolve_qualified_symbol(scope, &name, span)
                .ok_or_else(|| format!("undefined DATA initializer symbol `{name}`"))?,
        };
        if self.symbols.symbols[target.0].class == SymbolClass::Const {
            if self.symbols.symbols[target.0]
                .ty
                .as_ref()
                .is_some_and(|ty| ty.as_enum().is_some())
            {
                return Err("DATA requires an integer CONST bridge for enum values".into());
            }
            self.ensure_named_constant(target, span);
            let value = self
                .constants
                .get(&target)
                .copied()
                .ok_or("DATA requires an integer constant")?;
            let value = exact_const_value(value)
                .checked_add(i64::from(addend))
                .filter(|n| (0..=65535).contains(n))
                .ok_or("DATA constant is outside the byte/word range")?
                as u64;
            return match selector {
                Some(AddressByteSelector::Low) => Ok(Element::Bytes(vec![value as u8])),
                Some(AddressByteSelector::High) => Ok(Element::Bytes(vec![(value >> 8) as u8])),
                None if explicit_address => Ok(literal_address(value)),
                None => literal(value),
            };
        }
        if !matches!(
            self.symbols.symbols[target.0].class,
            SymbolClass::Data
                | SymbolClass::Var
                | SymbolClass::Array
                | SymbolClass::Proc
                | SymbolClass::Func
        ) {
            return Err("DATA address requires a storage label or routine".into());
        }
        let symbol = &self.symbols.symbols[target.0];
        let resolved = if symbol.scope == self.builtin_scope {
            let variable = crate::resident::resident_variable(&symbol.name)
                .ok_or("DATA address requires a declared storage label or routine")?;
            Target::Absolute(crate::target::AddressValue::data(u64::from(
                variable.address,
            )))
        } else {
            Target::Symbol(target)
        };
        Ok(Element::Address {
            target: resolved,
            selector,
            addend,
            width: self.data_address_width(target, selector),
        })
    }

    fn data_address_width(&self, target: SymbolId, selector: Option<AddressByteSelector>) -> u8 {
        if selector.is_some() {
            return 1;
        }
        let layout = TargetLayout::for_target(self.options.target);
        let pointer = if matches!(
            self.symbols.symbols[target.0].class,
            SymbolClass::Proc | SymbolClass::Func
        ) {
            layout.code_pointer
        } else {
            layout.data_pointer
        };
        pointer.size_bytes.get() as u8
    }
}
