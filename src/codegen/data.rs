use crate::lexer::{TokenKind, tokenize};

use super::*;

impl Generator {
    pub(super) fn emit_storage_initializers_with_source(
        &mut self,
        initializers: &[StorageInit],
        name: Option<String>,
        span: Span,
    ) {
        let start = self.current_absolute_address();
        for init in initializers {
            match init {
                StorageInit::Byte(byte) => self.emitter.emit_u8(*byte),
                StorageInit::LabelWord(label) => {
                    self.emitter.emit_u16_label(label, Span::new(0, 0));
                }
                StorageInit::Relocation {
                    kind,
                    target,
                    addend,
                    span,
                } => self.emit_storage_relocation(*kind, target, *addend, *span),
                StorageInit::Skip(range) => {
                    self.skipped_ranges.push(*range);
                    self.record_declaration_storage_ranges(
                        name.clone(),
                        span,
                        range.start,
                        range.start.wrapping_add(range.len),
                    );
                }
            }
        }
        self.record_declaration_storage_ranges(name, span, start, self.current_absolute_address());
    }

    pub(super) fn emit_array_backing_storage(&mut self) {
        let mut position = self.emitter.position();
        for backing in self.layout.array_backings.clone() {
            if let Err(diagnostic) = self.emitter.bind_label_at_position(
                backing.label.clone(),
                position,
                Span::new(0, 0),
            ) {
                self.diagnostics.push(diagnostic);
            }
            self.skipped_ranges.push(SkippedRange {
                start: self.emitter.origin.wrapping_add(position as u16),
                len: backing.size,
            });
            position = position.saturating_add(usize::from(backing.size));
        }
        self.bind_global_storage_labels();
    }

    fn emit_storage_relocation(
        &mut self,
        kind: StorageRelocationKind,
        target: &StorageRelocationTarget,
        addend: i32,
        span: Span,
    ) {
        let resolved = match target {
            StorageRelocationTarget::Absolute(address) => {
                Some(MachineSymbolAddress::Absolute(*address))
            }
            StorageRelocationTarget::OutputRelative(address) => {
                Some(MachineSymbolAddress::OutputRelative(*address))
            }
            StorageRelocationTarget::Label(label) => {
                Some(MachineSymbolAddress::Label(label.clone()))
            }
            StorageRelocationTarget::Name(name) => self.resolve_storage_relocation_name(name),
        };
        match resolved {
            Some(MachineSymbolAddress::Absolute(address)) => {
                let value = i32::from(address)
                    .checked_add(addend)
                    .and_then(|value| u16::try_from(value).ok());
                let Some(value) = value else {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        "static initializer address is outside the 16-bit address space",
                    ));
                    self.emitter.emit_zeroes(kind.width());
                    return;
                };
                match kind {
                    StorageRelocationKind::Low8 => self.emitter.emit_u8(value as u8),
                    StorageRelocationKind::High8 => self.emitter.emit_u8((value >> 8) as u8),
                    StorageRelocationKind::Word16 => self.emitter.emit_u16_le(value),
                }
            }
            Some(MachineSymbolAddress::OutputRelative(address)) => {
                let kind = match kind {
                    StorageRelocationKind::Low8 => CodegenRelocationKind::Low8,
                    StorageRelocationKind::High8 => CodegenRelocationKind::High8,
                    StorageRelocationKind::Word16 => CodegenRelocationKind::Word16,
                };
                if self
                    .emitter
                    .emit_output_relative(address, addend, kind)
                    .is_err()
                {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        "static initializer address is outside the 16-bit address space",
                    ));
                    self.emitter.emit_zeroes(kind.width());
                }
            }
            Some(MachineSymbolAddress::Label(label)) => match kind {
                StorageRelocationKind::Low8 => {
                    self.emitter.emit_u8_label_low_offset(label, addend, span)
                }
                StorageRelocationKind::High8 => {
                    self.emitter.emit_u8_label_high_offset(label, addend, span)
                }
                StorageRelocationKind::Word16 => {
                    self.emitter.emit_u16_label_offset(label, addend, span)
                }
            },
            None => {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    "static initializer target cannot be resolved by classic emission",
                ));
                self.emitter.emit_zeroes(kind.width());
            }
        }
    }

    fn resolve_storage_relocation_name(&self, name: &str) -> Option<MachineSymbolAddress> {
        let normalized = normalize_name(name);
        if let Some(target) = self.layout.machine_symbol_addresses.get(&normalized) {
            return Some(target.clone());
        }
        if let Some(slot) = self.layout.symbols.get(&normalized) {
            return Some(if slot.output_relative {
                MachineSymbolAddress::OutputRelative(slot.address)
            } else {
                MachineSymbolAddress::Absolute(slot.address)
            });
        }
        if let Some(routine) = self.routines.get(&normalized) {
            return Some(
                routine
                    .system_address
                    .map(MachineSymbolAddress::Absolute)
                    .unwrap_or_else(|| MachineSymbolAddress::Label(routine.label.clone())),
            );
        }
        if let Some(variable) = resident_variable(name) {
            return Some(MachineSymbolAddress::Absolute(variable.address));
        }
        Some(MachineSymbolAddress::Label(global_storage_label(name)))
    }

    fn bind_global_storage_labels(&mut self) {
        let mut symbols = self.layout.symbols.iter().collect::<Vec<_>>();
        symbols.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (name, slot) in symbols {
            let position = match self.layout.machine_symbol_addresses.get(name) {
                Some(
                    MachineSymbolAddress::Absolute(address)
                    | MachineSymbolAddress::OutputRelative(address),
                ) => usize::from(address.wrapping_sub(self.emitter.origin)),
                Some(MachineSymbolAddress::Label(label)) => {
                    let Some(position) = self.emitter.label_position(label) else {
                        self.diagnostics.push(Diagnostic::new(
                            Span::new(0, 0),
                            format!("array backing label `{label}` is unresolved"),
                        ));
                        continue;
                    };
                    position
                }
                None => usize::from(slot.address.wrapping_sub(self.emitter.origin)),
            };
            if let Err(diagnostic) = self.emitter.bind_label_at_position(
                global_storage_label(name),
                position,
                Span::new(0, 0),
            ) {
                self.diagnostics.push(diagnostic);
            }
        }
    }

    fn record_declaration_storage_ranges(
        &mut self,
        name: Option<String>,
        source_span: Span,
        start: u16,
        end: u16,
    ) {
        if end <= start {
            return;
        }
        self.record_source_range(
            CodegenSourceRangeKind::Declaration,
            name.clone(),
            source_span,
            start,
            end,
        );
        self.record_source_range(
            CodegenSourceRangeKind::StorageInitializer,
            name,
            source_span,
            start,
            end,
        );
    }

    pub(super) fn emit_string_literal_argument_storage(
        &mut self,
        args: &[Expr],
    ) -> Vec<Option<Absolute>> {
        args.iter()
            .map(|arg| {
                let ExprKind::String(text) = &arg.kind else {
                    return None;
                };
                Some(self.emit_string_literal_storage(text, arg.span))
            })
            .collect()
    }

    pub(super) fn emit_string_literal_storage(&mut self, text: &str, span: Span) -> Absolute {
        if (self.profile.enables_modern_optimizations() || self.preserve_modern_routine_layout)
            && let Some(address) = self
                .current_modern_routine_layout
                .string_literals
                .get(&StringLiteralKey::new(span, text))
                .copied()
        {
            return address;
        }

        let after_label = self.next_label("string_literal_after");
        let literal_address = self
            .emitter
            .origin
            .wrapping_add(self.emitter.position() as u16)
            .wrapping_add(3);
        if let Some(y) = self.straight_line_store_y.or(self.processor.y_immediate()) {
            self.label_store_y_hints.insert(after_label.clone(), y);
        }
        self.emit_jmp_label(after_label.clone(), span);
        let start = self.current_absolute_address();
        for byte in string_literal_storage(text) {
            self.emitter.emit_u8(byte);
        }
        let end = self.current_absolute_address();
        self.record_source_range(
            CodegenSourceRangeKind::StorageInitializer,
            Some("inline string literal".to_string()),
            span,
            start,
            end,
        );
        self.bind_codegen_label(after_label, span);
        Absolute::output_relative(literal_address)
    }

    pub(super) fn emit_string_literal_address_to_slot(
        &mut self,
        text: &str,
        span: Span,
        slot: StorageSlot,
    ) -> bool {
        if slot.size < 2 {
            return false;
        }
        let literal_address = self.emit_string_literal_storage(text, span);
        self.emit_store_constant(slot, literal_address.address());
        true
    }
}

fn global_storage_label(name: &str) -> String {
    format!("storage:{}", normalize_name(name))
}

pub(super) fn resolve_storage_initializer_targets(
    initializers: &mut [StorageInit],
    symbols: &HashMap<String, StorageSlot>,
    machine_symbols: &HashMap<String, MachineSymbolAddress>,
) {
    for initializer in initializers {
        let StorageInit::Relocation { target, .. } = initializer else {
            continue;
        };
        let StorageRelocationTarget::Name(name) = target else {
            continue;
        };
        let normalized = normalize_name(name);
        if let Some(machine) = machine_symbols.get(&normalized) {
            *target = match machine {
                MachineSymbolAddress::Absolute(address) => {
                    StorageRelocationTarget::Absolute(*address)
                }
                MachineSymbolAddress::OutputRelative(address) => {
                    StorageRelocationTarget::OutputRelative(*address)
                }
                MachineSymbolAddress::Label(label) => StorageRelocationTarget::Label(label.clone()),
            };
        } else if let Some(slot) = symbols.get(&normalized) {
            *target = if slot.output_relative {
                StorageRelocationTarget::OutputRelative(slot.address)
            } else {
                StorageRelocationTarget::Absolute(slot.address)
            };
        }
    }
}

pub(super) fn type_size(ty: &TypeRef) -> Option<u16> {
    match &ty.base {
        TypeBase::Fund(FundType::Byte | FundType::Char) => Some(1),
        TypeBase::Fund(FundType::Card | FundType::Int) => Some(2),
        TypeBase::Named(name) if is_string_type_name(name) => Some(1),
        TypeBase::Named(_) => None,
        TypeBase::Callable(_) => Some(2),
    }
}

pub(super) fn type_is_signed(ty: &TypeRef) -> bool {
    !ty.pointer && matches!(ty.base, TypeBase::Fund(FundType::Int))
}

pub(super) fn slot_signed_for_type(ty: &TypeRef) -> bool {
    matches!(ty.base, TypeBase::Fund(FundType::Int))
}

pub(super) fn constant_u16_with_defines(
    expr: &Expr,
    numeric_defines: &HashMap<String, u16>,
) -> Option<u16> {
    match &expr.kind {
        ExprKind::Name(name) => numeric_defines.get(&normalize_name(name)).copied(),
        ExprKind::Unary {
            op: UnaryOp::Plus,
            expr,
        } => constant_u16_with_defines(expr, numeric_defines),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } => Some(0u16.wrapping_sub(constant_u16_with_defines(expr, numeric_defines)?)),
        ExprKind::Binary { op, left, right } => {
            let left = constant_u16_with_defines(left, numeric_defines)?;
            let right = constant_u16_with_defines(right, numeric_defines)?;
            match op {
                BinaryOp::Add => Some(left.wrapping_add(right)),
                BinaryOp::Sub => Some(left.wrapping_sub(right)),
                BinaryOp::Mul => Some(left.wrapping_mul(right)),
                BinaryOp::Div if right != 0 => Some(left / right),
                BinaryOp::Mod if right != 0 => Some(left % right),
                BinaryOp::Lsh => Some(if right >= 16 {
                    0
                } else {
                    left.wrapping_shl(u32::from(right))
                }),
                BinaryOp::Rsh => Some(if right >= 16 {
                    0
                } else {
                    left.wrapping_shr(u32::from(right))
                }),
                BinaryOp::And => Some(left & right),
                BinaryOp::Or => Some(left | right),
                BinaryOp::Xor => Some(left ^ right),
                _ => None,
            }
        }
        _ => constant_u16(expr),
    }
}

pub(super) fn array_len_with_defines(
    entry: &DeclEntry,
    numeric_defines: &HashMap<String, u16>,
) -> Option<u16> {
    entry
        .size
        .as_ref()
        .and_then(|size| constant_u16_with_defines(size, numeric_defines))
}

pub(super) fn decl_is_array_like(decl: &VarDecl) -> bool {
    decl.storage == VarStorage::Array || is_string_type_ref(&decl.ty)
}

pub(super) fn is_string_type_ref(ty: &TypeRef) -> bool {
    matches!(&ty.base, TypeBase::Named(name) if is_string_type_name(name)) && !ty.pointer
}

pub(super) fn is_string_type_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("STRING")
}

pub(super) fn array_byte_size_with_defines(
    entry: &DeclEntry,
    element_size: u16,
    numeric_defines: &HashMap<String, u16>,
) -> u16 {
    if absolute_array_address_initializer(entry).is_some() {
        return 0;
    }
    if element_size == 1
        && let Some(bytes) = string_initializer_storage(entry)
    {
        return string_initialized_byte_size_with_defines(
            entry,
            bytes.len() as u16,
            numeric_defines,
        );
    }
    if let Some(initializers) = structured_array_initializer_storage(entry, element_size) {
        let initialized_size = storage_initializers_size(&initializers);
        return element_size
            .saturating_mul(
                array_len_with_defines(entry, numeric_defines)
                    .unwrap_or(initialized_size / element_size),
            )
            .max(initialized_size);
    }
    element_size.saturating_mul(array_len_with_defines(entry, numeric_defines).unwrap_or(1))
}

pub(super) fn array_entry_is_unsized_pointer_with_defines(
    entry: &DeclEntry,
    element_size: u16,
    numeric_defines: &HashMap<String, u16>,
) -> bool {
    array_len_with_defines(entry, numeric_defines).is_none()
        && absolute_array_address_initializer(entry).is_none()
        && symbolic_array_address_initializer(entry).is_none()
        && !(element_size == 1 && string_initializer_storage(entry).is_some())
        && structured_array_initializer_storage(entry, element_size).is_none()
}

pub(super) fn symbolic_array_address_initializer(entry: &DeclEntry) -> Option<String> {
    let ExprKind::Name(name) = &entry.initializer.as_ref()?.kind else {
        return None;
    };
    Some(format!("routine:{name}"))
}

pub(super) fn uninitialized_sized_byte_array_len_with_defines(
    entry: &DeclEntry,
    element_size: u16,
    numeric_defines: &HashMap<String, u16>,
) -> Option<u16> {
    (element_size == 1
        && absolute_array_address_initializer(entry).is_none()
        && string_initializer_storage(entry).is_none()
        && structured_array_initializer_storage(entry, element_size).is_none())
    .then(|| array_len_with_defines(entry, numeric_defines))
    .flatten()
}

pub(super) fn absolute_array_address_initializer(entry: &DeclEntry) -> Option<u16> {
    entry.size.as_ref()?;
    let initializer = entry.initializer.as_ref()?;
    match initializer.kind {
        ExprKind::Number(_) | ExprKind::Char(_) => constant_u16(initializer),
        ExprKind::Unary {
            op: UnaryOp::Plus | UnaryOp::Neg,
            ..
        } => constant_u16(initializer),
        _ => None,
    }
}

pub(super) fn absolute_alias_initializer(
    symbols: &HashMap<String, StorageSlot>,
    expr: &Expr,
) -> Option<u16> {
    absolute_alias_address_initializer(symbols, expr).map(Absolute::address)
}

pub(super) fn absolute_alias_address_initializer(
    symbols: &HashMap<String, StorageSlot>,
    expr: &Expr,
) -> Option<Absolute> {
    match &expr.kind {
        ExprKind::Number(_) | ExprKind::Char(_) => constant_u16(expr).map(Absolute::new),
        ExprKind::Unary {
            op: UnaryOp::Plus | UnaryOp::Neg,
            ..
        } => constant_u16(expr).map(Absolute::new),
        ExprKind::Name(name) => symbols
            .get(&normalize_name(name))
            .map(|slot| slot.absolute_byte(0)),
        ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } => absolute_alias_address_initializer(symbols, left)
            .zip(constant_u16(right))
            .map(|(base, offset)| base.offset(offset))
            .or_else(|| {
                constant_u16(left)
                    .zip(absolute_alias_address_initializer(symbols, right))
                    .map(|(offset, base)| base.offset(offset))
            }),
        ExprKind::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } => absolute_alias_address_initializer(symbols, left)
            .zip(constant_u16(right))
            .map(|(base, offset)| base.offset(0u16.wrapping_sub(offset))),
        _ => None,
    }
}

pub(super) fn string_initialized_byte_size_with_defines(
    entry: &DeclEntry,
    initialized_size: u16,
    numeric_defines: &HashMap<String, u16>,
) -> u16 {
    match array_len_with_defines(entry, numeric_defines) {
        Some(0) | None => initialized_size,
        Some(len) => len.saturating_add(1).max(initialized_size),
    }
}

pub(super) fn string_initializer_storage(entry: &DeclEntry) -> Option<Vec<u8>> {
    let ExprKind::String(text) = &entry.initializer.as_ref()?.kind else {
        return None;
    };
    Some(string_literal_storage(text))
}

pub(super) fn string_literal_storage(text: &str) -> Vec<u8> {
    let literal_bytes = source_string_bytes(text);
    let mut bytes = Vec::with_capacity(literal_bytes.len().saturating_add(1));
    bytes.push(literal_bytes.len() as u8);
    bytes.extend(literal_bytes);
    bytes
}

pub(super) fn source_string_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| source_char_byte(ch).unwrap_or(b'?'))
        .collect()
}

pub(super) fn numeric_array_initializer_storage(
    entry: &DeclEntry,
    element_size: u16,
) -> Option<Vec<u8>> {
    let values = numeric_array_initializer_values(entry)?;
    let mut bytes = Vec::with_capacity(values.len().saturating_mul(usize::from(element_size)));
    for value in values {
        bytes.push(value as u8);
        if element_size == 2 {
            bytes.push((value >> 8) as u8);
        } else if element_size != 1 {
            return None;
        }
    }
    Some(bytes)
}

pub(super) fn structured_array_initializer_storage(
    entry: &DeclEntry,
    element_size: u16,
) -> Option<Vec<StorageInit>> {
    if !matches!(element_size, 1 | 2) {
        return None;
    }
    let ExprKind::InitializerList(elements) = &entry.initializer.as_ref()?.kind else {
        return numeric_array_initializer_storage(entry, element_size)
            .map(|bytes| bytes.into_iter().map(StorageInit::Byte).collect());
    };
    let mut initializers = Vec::new();
    for element in elements {
        match &element.kind {
            InitializerElementKind::Literal { .. } => {
                let value = initializer_literal_value(element)?;
                initializers.push(StorageInit::Byte(value as u8));
                if element_size == 2 {
                    initializers.push(StorageInit::Byte((value >> 8) as u8));
                }
            }
            InitializerElementKind::Address {
                selector,
                target,
                addend,
            } => {
                let kind = match selector {
                    Some(AddressByteSelector::Low) => StorageRelocationKind::Low8,
                    Some(AddressByteSelector::High) => StorageRelocationKind::High8,
                    None => StorageRelocationKind::Word16,
                };
                if kind.width() != element_size {
                    return None;
                }
                initializers.push(StorageInit::Relocation {
                    kind,
                    target: StorageRelocationTarget::Name(target.clone()),
                    addend: *addend,
                    span: element.span,
                });
            }
            InitializerElementKind::Invalid => return None,
        }
    }
    Some(initializers)
}

pub(super) fn storage_initializers_size(initializers: &[StorageInit]) -> u16 {
    initializers.iter().fold(0u16, |size, initializer| {
        size.saturating_add(match initializer {
            StorageInit::Byte(_) => 1,
            StorageInit::LabelWord(_) => 2,
            StorageInit::Relocation { kind, .. } => kind.width(),
            StorageInit::Skip(range) => range.len,
        })
    })
}

pub(super) fn numeric_array_initializer_values(entry: &DeclEntry) -> Option<Vec<u16>> {
    let initializer = entry.initializer.as_ref()?;
    match &initializer.kind {
        ExprKind::InitializerList(elements) => elements
            .iter()
            .map(initializer_literal_value)
            .collect::<Option<Vec<_>>>(),
        ExprKind::Raw => {
            let text = initializer.text.trim();
            let inner = text.strip_prefix('[')?.strip_suffix(']')?;
            raw_initializer_values(inner)
        }
        _ => None,
    }
}

fn initializer_literal_value(element: &InitializerElement) -> Option<u16> {
    let InitializerElementKind::Literal { value, negative } = &element.kind else {
        return None;
    };
    let value = match value {
        InitializerLiteral::Number(number) => number.value?,
        InitializerLiteral::Char(ch) => u16::from(source_char_byte(*ch)?),
        InitializerLiteral::True => 1,
        InitializerLiteral::False | InitializerLiteral::Nil => 0,
    };
    Some(if *negative {
        0u16.wrapping_sub(value)
    } else {
        value
    })
}

fn raw_initializer_values(inner: &str) -> Option<Vec<u16>> {
    let mut values = Vec::new();
    let mut sign = 1i32;
    for token in tokenize(inner).ok()? {
        match token.kind {
            TokenKind::Eof | TokenKind::Comma => continue,
            TokenKind::Plus => {
                sign = 1;
                continue;
            }
            TokenKind::Minus => {
                sign = -1;
                continue;
            }
            _ => {}
        }
        let raw = parse_raw_initializer_value(&token.kind)?;
        let value = if sign < 0 {
            0u16.wrapping_sub(raw)
        } else {
            raw
        };
        values.push(value);
        sign = 1;
    }
    (!values.is_empty()).then_some(values)
}

fn parse_raw_initializer_value(token: &TokenKind) -> Option<u16> {
    match token {
        TokenKind::Number(number) => number.value,
        TokenKind::Char(ch) => source_char_byte(*ch).map(u16::from),
        TokenKind::Ident(name) => match normalize_name(name).as_str() {
            "TRUE" => Some(1),
            "FALSE" | "NIL" => Some(0),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn extend_entry_initializers(
    initializers: &mut Vec<StorageInit>,
    entry: &DeclEntry,
    total_size: u16,
    element_size: u16,
) {
    let entry_initializers = if let Some(bytes) = scalar_initializer_storage(entry, total_size) {
        bytes.into_iter().map(StorageInit::Byte).collect::<Vec<_>>()
    } else if element_size == 1 {
        string_initializer_storage(entry)
            .map(|bytes| bytes.into_iter().map(StorageInit::Byte).collect())
            .or_else(|| structured_array_initializer_storage(entry, element_size))
            .unwrap_or_default()
    } else {
        structured_array_initializer_storage(entry, element_size).unwrap_or_default()
    };
    let initialized_size = storage_initializers_size(&entry_initializers);
    initializers.extend(entry_initializers);
    let padding = usize::from(total_size.saturating_sub(initialized_size));
    initializers.extend(std::iter::repeat_n(StorageInit::Byte(0), padding));
}

pub(super) fn scalar_initializer_storage(entry: &DeclEntry, total_size: u16) -> Option<Vec<u8>> {
    let value = if let Some(value) = constant_u16(entry.initializer.as_ref()?) {
        value
    } else {
        let values = numeric_array_initializer_values(entry)?;
        if values.len() != 1 {
            return None;
        }
        values[0]
    };
    let immediate = Immediate::new(value);
    let mut bytes = Vec::with_capacity(usize::from(total_size.min(2)));
    if total_size > 0 {
        bytes.push(immediate.low());
    }
    if total_size > 1 {
        bytes.push(immediate.high());
    }
    Some(bytes)
}

pub(super) fn sized_byte_array_storage_bytes(byte_size: u16, len: u16) -> Vec<u8> {
    let mut bytes = vec![0; byte_size as usize];
    let len = Immediate::new(len);
    if bytes.len() > 2 {
        bytes[2] = len.low();
    }
    if bytes.len() > 3 {
        bytes[3] = len.high();
    }
    bytes
}

pub(super) fn fixed_array_pointer_storage(address: u16) -> [u8; 4] {
    let address = Immediate::new(address);
    [address.low(), address.high(), address.low(), address.high()]
}
