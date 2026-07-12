use crate::lexer::{NumberLiteral, TokenKind, tokenize};

use super::*;

impl Generator {
    pub(super) fn emit_machine_block(&mut self, items: &[MachineItem], span: Span) {
        let mut pending_operand_bytes = 0u8;
        self.emit_machine_items(items, span, &mut pending_operand_bytes, 0);
        self.record_current_unknown_effects();
        self.processor.invalidate_index_y();
        self.processor.invalidate_after_call();
        self.straight_line_store_y = None;
    }

    fn emit_machine_items(
        &mut self,
        items: &[MachineItem],
        span: Span,
        pending_operand_bytes: &mut u8,
        depth: u8,
    ) {
        if depth > 16 {
            self.diagnostics
                .push(Diagnostic::new(span, "recursive machine block DEFINE"));
            return;
        }

        let mut index = 0;
        while index < items.len() {
            if let Some((byte, split_item)) =
                self.split_compact_machine_number_item(&items[index], items.get(index + 1))
            {
                self.emit_machine_number(u16::from(byte), pending_operand_bytes);
                let diagnostic_count = self.diagnostics.len();
                self.emit_machine_items(&[split_item], span, pending_operand_bytes, depth + 1);
                if self.diagnostics.len() != diagnostic_count {
                    return;
                }
                index += 2;
                continue;
            }
            match &items[index] {
                MachineItem::Number(number) => {
                    if let Some(value) = number.value {
                        self.emit_machine_number(value, pending_operand_bytes);
                    } else {
                        self.diagnostics.push(Diagnostic::new(
                            span,
                            "machine block number is out of range",
                        ));
                        return;
                    }
                }
                MachineItem::StringLiteral(value) => {
                    let literal_address = self.current_absolute_address();
                    *pending_operand_bytes = 0;
                    for byte in string_literal_storage(value) {
                        self.emitter.emit_u8(byte);
                    }
                    self.emitter.emit_u16_le(literal_address);
                }
                MachineItem::CharLiteral(value) => {
                    *pending_operand_bytes = 0;
                    let byte = crate::source::source_char_byte(*value).unwrap_or(0);
                    self.emitter.emit_u8(byte);
                    self.emitter.emit_u8(0x9A);
                }
                MachineItem::Name(name) => {
                    let (offset, consumed) = machine_block_name_offset(&items[index + 1..]);
                    if offset == 0
                        && let Some(items) =
                            self.machine_defines.get(&normalize_name(name)).cloned()
                    {
                        let diagnostic_count = self.diagnostics.len();
                        self.emit_machine_items(&items, span, pending_operand_bytes, depth + 1);
                        if self.diagnostics.len() != diagnostic_count {
                            return;
                        }
                    } else if offset == 0
                        && let Some(value) = self.numeric_defines.get(&normalize_name(name))
                    {
                        self.emit_machine_number(*value, pending_operand_bytes);
                    } else if let Some(address) = self.machine_symbol_address(name) {
                        if let Err(message) = self.emit_machine_symbol_address(
                            address,
                            i32::from(offset),
                            None,
                            pending_operand_bytes,
                            span,
                            name,
                            true,
                        ) {
                            self.diagnostics.push(Diagnostic::new(span, message));
                            return;
                        }
                        index += consumed;
                    } else if offset == 0 {
                        if let Some(routine) = self.routines.get(&normalize_name(name)).cloned() {
                            if let Some(address) = routine.system_address {
                                self.emit_machine_absolute(address, pending_operand_bytes);
                            } else {
                                *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
                                self.emitter.emit_u16_label(routine.label, span);
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::new(
                                span,
                                format!("unknown machine block symbol `{name}`"),
                            ));
                            return;
                        }
                    } else {
                        self.diagnostics.push(Diagnostic::new(
                            span,
                            format!("machine block offset is not supported for `{name}`"),
                        ));
                        return;
                    }
                }
                MachineItem::AddressByte { selector, name } => {
                    if let Some(value) = self.numeric_defines.get(&normalize_name(name)) {
                        let byte = match selector {
                            AddressByteSelector::Low => Immediate::new(*value).low(),
                            AddressByteSelector::High => Immediate::new(*value).high(),
                        };
                        self.emitter.emit_u8(byte);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    } else if let Some(address) = self.machine_symbol_address(name) {
                        if let Err(message) = self.emit_machine_symbol_address(
                            address,
                            0,
                            Some(*selector),
                            pending_operand_bytes,
                            span,
                            name,
                            false,
                        ) {
                            self.diagnostics.push(Diagnostic::new(span, message));
                            return;
                        }
                    } else if let Some(routine) = self.routines.get(&normalize_name(name)).cloned()
                    {
                        if let Some(address) = routine.system_address {
                            let byte = match selector {
                                AddressByteSelector::Low => Immediate::new(address).low(),
                                AddressByteSelector::High => Immediate::new(address).high(),
                            };
                            self.emitter.emit_u8(byte);
                            *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                        } else {
                            match selector {
                                AddressByteSelector::Low => {
                                    self.emitter.emit_u8_label_low(routine.label, span)
                                }
                                AddressByteSelector::High => {
                                    self.emitter.emit_u8_label_high(routine.label, span)
                                }
                            }
                            *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                        }
                    } else {
                        self.diagnostics.push(Diagnostic::new(
                            span,
                            format!("unknown machine block symbol `{name}`"),
                        ));
                        return;
                    }
                }
                MachineItem::AddressExpr(expr) => {
                    if let Err(message) =
                        self.emit_machine_address_expr(expr, pending_operand_bytes, span)
                    {
                        self.diagnostics.push(Diagnostic::new(span, message));
                        return;
                    }
                }
                MachineItem::Raw(raw) if raw == "," => {}
                MachineItem::Raw(raw) if raw.starts_with('"') && raw.ends_with('"') => {
                    let literal_address = self.current_absolute_address();
                    *pending_operand_bytes = 0;
                    for byte in string_literal_storage(&raw[1..raw.len().saturating_sub(1)]) {
                        self.emitter.emit_u8(byte);
                    }
                    self.emitter.emit_u16_le(literal_address);
                }
                MachineItem::Raw(raw) if raw.starts_with('\'') && raw.ends_with('\'') => {
                    *pending_operand_bytes = 0;
                    let value = raw[1..raw.len().saturating_sub(1)]
                        .bytes()
                        .next()
                        .unwrap_or(0);
                    self.emitter.emit_u8(value);
                    self.emitter.emit_u8(0x9A);
                }
                MachineItem::Raw(_) => {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        "machine block item is not supported yet",
                    ));
                    return;
                }
            }
            index += 1;
        }
    }

    fn split_compact_machine_number_item(
        &self,
        item: &MachineItem,
        next: Option<&MachineItem>,
    ) -> Option<(u8, MachineItem)> {
        let MachineItem::Number(number) = item else {
            return None;
        };
        let digits = number.text.strip_prefix('$')?;
        if digits.len() <= 2 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let byte = u8::from_str_radix(&digits[..2], 16).ok()?;
        match next? {
            MachineItem::Name(suffix) => {
                let name = format!("{}{suffix}", &digits[2..]);
                self.machine_symbol_name_is_known(&name)
                    .then_some((byte, MachineItem::Name(name)))
            }
            MachineItem::AddressExpr(expr) => {
                let MachineAddressAtom::Name(suffix) = &expr.atom else {
                    return None;
                };
                let name = format!("{}{suffix}", &digits[2..]);
                self.machine_symbol_name_is_known(&name).then(|| {
                    (
                        byte,
                        MachineItem::AddressExpr(MachineAddressExpr {
                            selector: expr.selector,
                            explicit_address: expr.explicit_address,
                            atom: MachineAddressAtom::Name(name),
                            offset: expr.offset,
                            text: format!("{}{}", &digits[2..], expr.text),
                        }),
                    )
                })
            }
            _ => None,
        }
    }

    fn machine_symbol_name_is_known(&self, name: &str) -> bool {
        let normalized = normalize_name(name);
        self.machine_defines.contains_key(&normalized)
            || self.numeric_defines.contains_key(&normalized)
            || self.machine_symbol_address(name).is_some()
            || self.routines.contains_key(&normalized)
    }

    pub(super) fn record_machine_block_analysis(
        &mut self,
        items: &[MachineItem],
        span: Span,
        start: u16,
    ) {
        let effects = self.machine_block_effects(items);
        self.machine_blocks.push(CodegenMachineBlockAnalysis {
            routine: self.current_routine_name(),
            source_span: span,
            address: start,
            trusted: false,
            summary: format_machine_block_effect_summary(effects),
        });
    }

    fn machine_block_effects(&self, items: &[MachineItem]) -> RoutineEffects {
        let Some(bytes) = self.machine_block_bytes_for_effects(items) else {
            return RoutineEffects::unknown();
        };
        let mut effects = RoutineEffects::known_empty();
        let mut offset = 0usize;
        while offset < bytes.len() {
            let opcode = bytes[offset];
            let Some(instruction) = decode_instruction(opcode) else {
                return RoutineEffects::unknown();
            };
            let len = instruction.len;
            if offset + len > bytes.len() {
                return RoutineEffects::unknown();
            }
            match opcode {
                opcode::JSR_ABS | opcode::JMP_ABS | 0x6C | 0x40 => {
                    return RoutineEffects::unknown();
                }
                opcode::STA_ZP
                | opcode::STX_ZP
                | opcode::STY_ZP
                | opcode::INC_ZP
                | opcode::DEC_ZP
                | 0x06
                | 0x26
                | 0x46
                | 0x66
                | opcode::STA_ZP_X
                | 0x94
                | 0x96
                | 0xD6
                | 0xF6
                | 0x16
                | 0x36
                | 0x56
                | 0x76 => effects.record_zero_page_write(ZeroPage::new(bytes[offset + 1])),
                opcode::STA_ABS
                | opcode::STX_ABS
                | opcode::STY_ABS
                | opcode::INC_ABS
                | opcode::DEC_ABS
                | opcode::ASL_ABS
                | opcode::ROL_ABS
                | opcode::LSR_ABS
                | opcode::ROR_ABS => {
                    let address = u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]);
                    effects.record_absolute_write(address, 1);
                }
                opcode::STA_ABS_X | 0x9E | 0xDE | 0xFE | 0x1E | 0x3E | 0x5E | 0x7E => {
                    let address = u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]);
                    if address < 0x100 {
                        return RoutineEffects::unknown();
                    }
                    effects.record_unknown_absolute_write();
                }
                _ => {}
            }
            offset += len;
        }
        effects
    }

    fn machine_block_bytes_for_effects(&self, items: &[MachineItem]) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut index = 0usize;
        let mut pending_operand_bytes = 0u8;
        while index < items.len() {
            match &items[index] {
                MachineItem::Number(number) => {
                    let value = number.value?;
                    push_machine_effect_number(&mut bytes, value, &mut pending_operand_bytes);
                }
                MachineItem::StringLiteral(value) => {
                    pending_operand_bytes = 0;
                    bytes.extend(string_literal_storage(value));
                    bytes.extend([0, 0]);
                }
                MachineItem::CharLiteral(value) => {
                    pending_operand_bytes = 0;
                    bytes.push(crate::source::source_char_byte(*value).unwrap_or(0));
                    bytes.push(0x9A);
                }
                MachineItem::Name(name) => {
                    let (offset, consumed) = machine_block_name_offset(&items[index + 1..]);
                    if offset == 0
                        && let Some(value) = self.numeric_defines.get(&normalize_name(name))
                    {
                        push_machine_effect_number(&mut bytes, *value, &mut pending_operand_bytes);
                    } else if let Some(MachineSymbolAddress::Absolute(value)) =
                        self.machine_symbol_address(name)
                    {
                        push_machine_effect_absolute(
                            &mut bytes,
                            value.wrapping_add(offset),
                            &mut pending_operand_bytes,
                        );
                        index += consumed;
                    } else if offset == 0 {
                        let routine = self.routines.get(&normalize_name(name))?;
                        let address = routine.system_address?;
                        push_machine_effect_absolute(
                            &mut bytes,
                            address,
                            &mut pending_operand_bytes,
                        );
                    } else {
                        return None;
                    }
                }
                MachineItem::AddressByte { selector, name } => {
                    if let Some(value) = self.numeric_defines.get(&normalize_name(name)) {
                        bytes.push(match selector {
                            AddressByteSelector::Low => Immediate::new(*value).low(),
                            AddressByteSelector::High => Immediate::new(*value).high(),
                        });
                    } else if let Some(MachineSymbolAddress::Absolute(value)) =
                        self.machine_symbol_address(name)
                    {
                        bytes.push(match selector {
                            AddressByteSelector::Low => Immediate::new(value).low(),
                            AddressByteSelector::High => Immediate::new(value).high(),
                        });
                    } else {
                        let routine = self.routines.get(&normalize_name(name))?;
                        let address = routine.system_address?;
                        bytes.push(match selector {
                            AddressByteSelector::Low => Immediate::new(address).low(),
                            AddressByteSelector::High => Immediate::new(address).high(),
                        });
                    }
                    pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                }
                MachineItem::AddressExpr(expr) => {
                    push_machine_effect_address_expr(
                        self,
                        &mut bytes,
                        expr,
                        &mut pending_operand_bytes,
                    )?;
                }
                MachineItem::Raw(raw) if raw == "," => {}
                MachineItem::Raw(raw) if raw.starts_with('"') && raw.ends_with('"') => {
                    pending_operand_bytes = 0;
                    bytes.extend(string_literal_storage(&raw[1..raw.len().saturating_sub(1)]));
                    bytes.extend([0, 0]);
                }
                MachineItem::Raw(raw) if raw.starts_with('\'') && raw.ends_with('\'') => {
                    pending_operand_bytes = 0;
                    bytes.push(
                        raw[1..raw.len().saturating_sub(1)]
                            .bytes()
                            .next()
                            .unwrap_or(0),
                    );
                    bytes.push(0x9A);
                }
                MachineItem::Raw(_) => return None,
            }
            index += 1;
        }
        Some(bytes)
    }

    fn emit_machine_number(&mut self, value: u16, pending_operand_bytes: &mut u8) {
        if value > 0xFF {
            self.emitter.emit_u16_le(value);
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
            return;
        }

        let byte = Immediate::new(value).low();
        self.emitter.emit_u8(byte);
        if *pending_operand_bytes > 0 {
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
        } else {
            *pending_operand_bytes = machine_opcode_operand_bytes(byte);
        }
    }

    fn emit_machine_absolute(&mut self, address: u16, pending_operand_bytes: &mut u8) {
        if *pending_operand_bytes == 1 {
            self.emitter.emit_u8(Immediate::new(address).low());
            *pending_operand_bytes = 0;
        } else {
            self.emitter.emit_u16_le(address);
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
        }
    }

    fn machine_symbol_address(&self, name: &str) -> Option<MachineSymbolAddress> {
        let normalized = normalize_name(name);
        self.layout
            .machine_symbol_addresses
            .get(&normalized)
            .cloned()
            .or_else(|| {
                self.lookup_slot(name)
                    .map(|slot| MachineSymbolAddress::Absolute(slot.address))
            })
    }

    fn emit_machine_symbol_address(
        &mut self,
        address: MachineSymbolAddress,
        offset: i32,
        selector: Option<AddressByteSelector>,
        pending_operand_bytes: &mut u8,
        span: Span,
        text: &str,
        force_absolute_operand: bool,
    ) -> Result<(), String> {
        match address {
            MachineSymbolAddress::Absolute(value) => {
                let value = machine_apply_offset(value, offset, text)?;
                if selector.is_none() && force_absolute_operand {
                    self.emit_machine_absolute(value, pending_operand_bytes);
                } else {
                    self.emit_machine_resolved_address_value(
                        value,
                        selector,
                        pending_operand_bytes,
                    );
                }
            }
            MachineSymbolAddress::Label(label) => {
                if offset != 0 {
                    let Some(position) = self.emitter.label_position(&label) else {
                        return Err(format!(
                            "machine block item `{text}` with offset is not relocatable yet"
                        ));
                    };
                    let value = self
                        .emitter
                        .origin
                        .wrapping_add(u16::try_from(position).unwrap_or(u16::MAX));
                    let value = machine_apply_offset(value, offset, text)?;
                    if selector.is_none() && force_absolute_operand {
                        self.emit_machine_absolute(value, pending_operand_bytes);
                    } else {
                        self.emit_machine_resolved_address_value(
                            value,
                            selector,
                            pending_operand_bytes,
                        );
                    }
                    return Ok(());
                }
                match selector {
                    Some(AddressByteSelector::Low) => {
                        self.emitter.emit_u8_label_low(label, span);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    }
                    Some(AddressByteSelector::High) => {
                        self.emitter.emit_u8_label_high(label, span);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    }
                    None if *pending_operand_bytes == 1 => {
                        self.emitter.emit_u8_label_low(label, span);
                        *pending_operand_bytes = 0;
                    }
                    None => {
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
                        self.emitter.emit_u16_label(label, span);
                    }
                }
            }
        }
        Ok(())
    }

    fn emit_machine_address_expr(
        &mut self,
        expr: &MachineAddressExpr,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) -> Result<(), String> {
        let offset = self.machine_address_expr_offset(expr)?;
        match &expr.atom {
            MachineAddressAtom::Number(number) => {
                let value = machine_number_with_offset(number, offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
                Ok(())
            }
            MachineAddressAtom::Name(name) => self.emit_machine_named_address_expr(
                name,
                expr,
                offset,
                pending_operand_bytes,
                span,
            ),
            MachineAddressAtom::Current => {
                let value =
                    machine_apply_offset(self.current_absolute_address(), offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
                Ok(())
            }
        }
    }

    fn machine_address_expr_offset(&self, expr: &MachineAddressExpr) -> Result<i32, String> {
        let mut offset = expr.offset;
        if let Some((negative, name)) = machine_address_symbolic_offset(&expr.text) {
            let Some(value) = self.numeric_defines.get(&normalize_name(name)) else {
                return Err(format!(
                    "machine block item `{}` references unknown numeric define `{name}`",
                    expr.text
                ));
            };
            let value = i32::from(*value);
            offset = offset.wrapping_add(if negative { -value } else { value });
        }
        Ok(offset)
    }

    fn emit_machine_named_address_expr(
        &mut self,
        name: &str,
        expr: &MachineAddressExpr,
        offset: i32,
        pending_operand_bytes: &mut u8,
        span: Span,
    ) -> Result<(), String> {
        let normalized = normalize_name(name);
        if machine_address_expr_uses_caret(expr) {
            let Some(value) = self.machine_caret_symbol_value(name) else {
                return Err(format!(
                    "machine block item `{}` cannot be resolved to a compile-time pointer value",
                    expr.text
                ));
            };
            let value = machine_apply_offset(value, offset, &expr.text)?;
            self.emit_machine_resolved_address_value(value, expr.selector, pending_operand_bytes);
        } else if let Some(value) = self.numeric_defines.get(&normalized) {
            let value = machine_apply_offset(*value, offset, &expr.text)?;
            self.emit_machine_resolved_address_value(value, expr.selector, pending_operand_bytes);
        } else if let Some(address) = self.machine_symbol_address(name) {
            self.emit_machine_symbol_address(
                address,
                offset,
                expr.selector,
                pending_operand_bytes,
                span,
                &expr.text,
                false,
            )?;
        } else if let Some(routine) = self.routines.get(&normalized).cloned() {
            if let Some(address) = routine.system_address {
                let value = machine_apply_offset(address, offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
            } else if offset == 0 {
                match expr.selector {
                    Some(AddressByteSelector::Low) => {
                        self.emitter.emit_u8_label_low(routine.label, span);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    }
                    Some(AddressByteSelector::High) => {
                        self.emitter.emit_u8_label_high(routine.label, span);
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
                    }
                    None => {
                        *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
                        self.emitter.emit_u16_label(routine.label, span);
                    }
                }
            } else if let Some(position) = self.emitter.label_position(&routine.label) {
                let value = self
                    .emitter
                    .origin
                    .wrapping_add(u16::try_from(position).unwrap_or(u16::MAX));
                let value = machine_apply_offset(value, offset, &expr.text)?;
                self.emit_machine_resolved_address_value(
                    value,
                    expr.selector,
                    pending_operand_bytes,
                );
            } else {
                return Err(format!(
                    "machine block item `{}` with offset is not relocatable yet",
                    expr.text
                ));
            }
        } else {
            return Err(format!("unknown machine block symbol `{name}`"));
        }
        Ok(())
    }

    fn machine_caret_symbol_value(&self, name: &str) -> Option<u16> {
        self.layout
            .machine_caret_values
            .get(&normalize_name(name))
            .copied()
    }

    fn emit_machine_resolved_address_value(
        &mut self,
        value: u16,
        selector: Option<AddressByteSelector>,
        pending_operand_bytes: &mut u8,
    ) {
        match selector {
            Some(AddressByteSelector::Low) => {
                self.emitter.emit_u8(Immediate::new(value).low());
                *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
            }
            Some(AddressByteSelector::High) => {
                self.emitter.emit_u8(Immediate::new(value).high());
                *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
            }
            None if value <= 0xFF => self.emit_machine_number(value, pending_operand_bytes),
            None => self.emit_machine_absolute(value, pending_operand_bytes),
        }
    }
}

pub(super) fn format_machine_block_effect_summary(effects: RoutineEffects) -> String {
    if !effects.known {
        return "advisory: opaque/unsupported machine block".to_string();
    }
    let mut parts = Vec::new();
    let zero_page_writes = format_zero_page_writes(effects);
    if !zero_page_writes.is_empty() {
        parts.push(format!("zp writes {}", zero_page_writes.join(",")));
    }
    let absolute_writes = effects
        .absolute_writes
        .iter()
        .flatten()
        .map(|range| {
            if range.size <= 1 {
                format!("${:04X}", range.address)
            } else {
                format!(
                    "${:04X}-${:04X}",
                    range.address,
                    range.address.wrapping_add(range.size - 1)
                )
            }
        })
        .collect::<Vec<_>>();
    if !absolute_writes.is_empty() {
        parts.push(format!("absolute writes {}", absolute_writes.join(",")));
    }
    if effects.writes_unknown_absolute {
        parts.push("unknown absolute write".to_string());
    }
    if parts.is_empty() {
        "advisory: no memory writes observed".to_string()
    } else {
        format!("advisory: {}", parts.join("; "))
    }
}

pub(super) fn collect_machine_defines(program: &Program) -> HashMap<String, Vec<MachineItem>> {
    let mut defines = HashMap::new();
    for module in &program.modules {
        for item in &module.items {
            let Item::Define(define) = item else {
                continue;
            };
            for entry in &define.entries {
                if let Some(items) = parse_machine_define_value(&entry.value) {
                    defines.insert(normalize_name(&entry.name), items);
                }
            }
        }
    }
    defines
}

pub(super) fn parse_machine_define_value(value: &str) -> Option<Vec<MachineItem>> {
    let tokens = tokenize(value).ok()?;
    let tokens = tokens
        .into_iter()
        .filter(|token| !matches!(token.kind, TokenKind::Eof))
        .collect::<Vec<_>>();
    if !matches!(tokens.first()?.kind, TokenKind::LBracket) {
        if tokens.len() != 1 {
            return None;
        }
        let item = match &tokens.first()?.kind {
            TokenKind::Number(number) => MachineItem::Number(number.clone()),
            TokenKind::String(value) => MachineItem::StringLiteral(value.clone()),
            TokenKind::Char(value) => MachineItem::CharLiteral(*value),
            _ => return None,
        };
        return Some(vec![item]);
    }

    let mut items = Vec::new();
    let mut index = 1usize;
    while let Some(token) = tokens.get(index) {
        match &token.kind {
            TokenKind::RBracket => return Some(items),
            TokenKind::Number(number) => items.push(MachineItem::Number(number.clone())),
            TokenKind::String(value) => items.push(MachineItem::StringLiteral(value.clone())),
            TokenKind::Char(value) => items.push(MachineItem::CharLiteral(*value)),
            TokenKind::Ident(name) => items.push(MachineItem::Name(name.clone())),
            TokenKind::Lt | TokenKind::Gt
                if matches!(
                    tokens.get(index + 1).map(|token| &token.kind),
                    Some(TokenKind::Ident(_))
                ) =>
            {
                let selector = if matches!(token.kind, TokenKind::Lt) {
                    AddressByteSelector::Low
                } else {
                    AddressByteSelector::High
                };
                let Some(TokenKind::Ident(name)) = tokens.get(index + 1).map(|token| &token.kind)
                else {
                    return None;
                };
                items.push(MachineItem::AddressByte {
                    selector,
                    name: name.clone(),
                });
                index += 1;
            }
            TokenKind::Comma => items.push(MachineItem::Raw(",".to_string())),
            _ => return None,
        }
        index += 1;
    }
    None
}

fn machine_block_name_offset(items: &[MachineItem]) -> (u16, usize) {
    let [
        MachineItem::Raw(op),
        MachineItem::Number(NumberLiteral {
            value: Some(value), ..
        }),
        ..,
    ] = items
    else {
        return (0, 0);
    };
    match op.as_str() {
        "+" => (*value, 2),
        "-" => (0u16.wrapping_sub(*value), 2),
        _ => (0, 0),
    }
}

fn machine_opcode_operand_bytes(opcode: u8) -> u8 {
    decode_instruction(opcode)
        .map(|instruction| instruction.len.saturating_sub(1) as u8)
        .unwrap_or(0)
}

fn push_machine_effect_number(bytes: &mut Vec<u8>, value: u16, pending_operand_bytes: &mut u8) {
    if value > 0xFF {
        bytes.extend(value.to_le_bytes());
        *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
        return;
    }

    let byte = Immediate::new(value).low();
    bytes.push(byte);
    if *pending_operand_bytes > 0 {
        *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
    } else {
        *pending_operand_bytes = machine_opcode_operand_bytes(byte);
    }
}

fn push_machine_effect_absolute(bytes: &mut Vec<u8>, address: u16, pending_operand_bytes: &mut u8) {
    let immediate = Immediate::new(address);
    if *pending_operand_bytes == 1 {
        bytes.push(immediate.low());
        *pending_operand_bytes = 0;
    } else {
        bytes.extend(address.to_le_bytes());
        *pending_operand_bytes = pending_operand_bytes.saturating_sub(2);
    }
}

fn push_machine_effect_address_expr(
    generator: &Generator,
    bytes: &mut Vec<u8>,
    expr: &MachineAddressExpr,
    pending_operand_bytes: &mut u8,
) -> Option<()> {
    let value = match &expr.atom {
        MachineAddressAtom::Number(number) => {
            let offset = generator.machine_address_expr_offset(expr).ok()?;
            machine_number_with_offset(number, offset, &expr.text).ok()?
        }
        MachineAddressAtom::Name(name) => {
            let offset = generator.machine_address_expr_offset(expr).ok()?;
            if machine_address_expr_uses_caret(expr) {
                let value = generator.machine_caret_symbol_value(name)?;
                machine_apply_offset(value, offset, &expr.text).ok()?
            } else if let Some(value) = generator.numeric_defines.get(&normalize_name(name)) {
                machine_apply_offset(*value, offset, &expr.text).ok()?
            } else if let Some(MachineSymbolAddress::Absolute(value)) =
                generator.machine_symbol_address(name)
            {
                machine_apply_offset(value, offset, &expr.text).ok()?
            } else {
                let routine = generator.routines.get(&normalize_name(name))?;
                let address = routine.system_address?;
                machine_apply_offset(address, offset, &expr.text).ok()?
            }
        }
        MachineAddressAtom::Current => {
            let offset = generator.machine_address_expr_offset(expr).ok()?;
            machine_apply_offset(generator.current_absolute_address(), offset, &expr.text).ok()?
        }
    };
    match expr.selector {
        Some(AddressByteSelector::Low) => {
            bytes.push(Immediate::new(value).low());
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
        }
        Some(AddressByteSelector::High) => {
            bytes.push(Immediate::new(value).high());
            *pending_operand_bytes = pending_operand_bytes.saturating_sub(1);
        }
        None if value <= 0xFF => push_machine_effect_number(bytes, value, pending_operand_bytes),
        None => push_machine_effect_absolute(bytes, value, pending_operand_bytes),
    }
    Some(())
}

fn machine_address_expr_uses_caret(expr: &MachineAddressExpr) -> bool {
    expr.text.contains('^')
}

fn machine_number_with_offset(
    number: &NumberLiteral,
    offset: i32,
    text: &str,
) -> Result<u16, String> {
    let value = number
        .value
        .ok_or_else(|| format!("machine block item `{text}` does not fit in 16 bits"))?;
    machine_apply_offset(value, offset, text)
}

fn machine_apply_offset(value: u16, offset: i32, text: &str) -> Result<u16, String> {
    let offset = u16::try_from(offset.rem_euclid(0x1_0000))
        .map_err(|_| format!("machine block item `{text}` does not fit in 16 bits"))?;
    Ok(value.wrapping_add(offset))
}
