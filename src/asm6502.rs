//! Small, integrated NMOS 6502 assembler used by Action! `ASM` blocks.
//!
//! This deliberately implements a useful MADS-compatible subset. It produces
//! the same relocatable machine items used by both code generators while the
//! source-level statement remains distinct from a legacy Action! machine
//! block.

use std::collections::HashMap;

use crate::ast::{
    AddressByteSelector, MachineAddressAtom, MachineAddressExpr, MachineItem, QualifiedName,
};
use crate::codegen::{AddressingMode, decode_6502_opcode};
use crate::diagnostic::Diagnostic;
use crate::lexer::{NumberKind, NumberLiteral, decode_atascii_escape};
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineAsmMode {
    Analyzed,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineAsmProgram {
    pub items: Vec<MachineItem>,
    pub bytes: Vec<u8>,
    pub relocations: Vec<InlineAsmRelocation>,
    pub source: String,
    pub mode: InlineAsmMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineAsmRelocationKind {
    Absolute16,
    /// An unselected one-byte constant. Unlike `Low8`, this must fit without
    /// truncation.
    Byte8,
    Low8,
    High8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineAsmRelocationTarget {
    Symbol(String),
    InlineOffset(u16),
    Absolute(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineAsmSymbolUse {
    Address,
    Constant,
    Read,
    Write,
    ReadWrite,
    IndexedRead,
    IndexedWrite,
    IndexedReadWrite,
    Call,
    Control,
    PointerRead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineAsmRelocation {
    pub offset: u16,
    pub kind: InlineAsmRelocationKind,
    pub target: InlineAsmRelocationTarget,
    pub addend: i32,
    pub requires_zero_page: bool,
    pub symbol_use: InlineAsmSymbolUse,
    pub span: Span,
}

/// Target-specific machine-state summary computed only when lowering the
/// target-neutral inline-code payload into MIR6502.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InlineAsmMachineState {
    pub reads: InlineAsmRegisterSet,
    pub clobbers: InlineAsmRegisterSet,
    pub stack_depth_delta: Option<i8>,
    pub stack_balanced_at_exits: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InlineAsmRegisterSet {
    pub a: bool,
    pub x: bool,
    pub y: bool,
    pub flags: bool,
    pub sp: bool,
}

#[derive(Debug, Clone)]
struct ParsedInstruction {
    mnemonic: String,
    operand: String,
    span: Span,
    offset: usize,
    line_index: usize,
    opcode: u8,
    mode: AddressingMode,
}

#[derive(Debug, Clone)]
struct SourceLine {
    text: String,
    span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeSuffix {
    Default,
    ZeroPage,
    Absolute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprAtom {
    Number(i32),
    Name(String),
    ExternalName(String),
    Current,
    AnonymousForward,
    AnonymousBackward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AsmExpr {
    selector: Option<AddressByteSelector>,
    atom: ExprAtom,
    addend: i32,
}

#[derive(Debug, Clone)]
struct PendingInstruction {
    mnemonic: String,
    suffix: SizeSuffix,
    operand: String,
    operand_label: Option<String>,
    span: Span,
    line_index: usize,
}

pub fn assemble(
    source: &str,
    source_offset: usize,
    mode: InlineAsmMode,
) -> Result<InlineAsmProgram, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let sanitized = match strip_asm_comments(source, source_offset) {
        Ok(source) => source,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            source.to_string()
        }
    };
    let lines = source_lines(&sanitized, source_offset);
    let mut constants = HashMap::<String, i32>::new();
    let mut labels = HashMap::<String, usize>::new();
    let mut anonymous_labels = Vec::<(usize, usize)>::new();
    let mut instructions = Vec::<ParsedInstruction>::new();
    let mut offset = 0usize;

    for (line_index, line) in lines.iter().enumerate() {
        let mut text = line.text.trim().to_string();
        if text.is_empty() {
            continue;
        }

        if let Some((label, rest)) = split_label(&text) {
            if label == "@" {
                anonymous_labels.push((line_index, offset));
            } else {
                let key = normalize(&label);
                if labels.insert(key, offset).is_some() {
                    diagnostics.push(Diagnostic::new(
                        line.span,
                        format!("duplicate inline assembler label `{label}`"),
                    ));
                }
            }
            text = rest.trim().to_string();
            if text.is_empty() {
                continue;
            }
        }

        if let Some((name, expression)) = split_constant(&text) {
            match parse_expression(expression, &constants, line.span) {
                Ok(expr) => match resolve_numeric_expr(&expr, &constants) {
                    Some(value) => {
                        constants.insert(normalize(name), value);
                    }
                    None => diagnostics.push(Diagnostic::new(
                        line.span,
                        "inline assembler constants must be numeric and previously defined",
                    )),
                },
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
            continue;
        }

        if let Some((label, rest)) = split_optional_label(&text) {
            let key = normalize(label);
            if labels.insert(key, offset).is_some() {
                diagnostics.push(Diagnostic::new(
                    line.span,
                    format!("duplicate inline assembler label `{label}`"),
                ));
            }
            text = rest.trim().to_string();
            if text.is_empty() {
                continue;
            }
        }

        let pending = match parse_instruction_line(&text, line.span, line_index) {
            Ok(instruction) => instruction,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                continue;
            }
        };
        if let Some(label) = &pending.operand_label
            && pending.operand.is_empty()
        {
            diagnostics.push(Diagnostic::new(
                line.span,
                format!(
                    "self-modification label `{label}` requires an instruction with an encoded operand"
                ),
            ));
            continue;
        }
        let (opcode, addressing) = match select_instruction(&pending, &constants, line.span) {
            Ok(selection) => selection,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                continue;
            }
        };
        if let Some(label) = &pending.operand_label {
            if addressing_len(addressing) == 1 {
                diagnostics.push(Diagnostic::new(
                    line.span,
                    format!(
                        "self-modification label `{label}` requires an instruction with an encoded operand"
                    ),
                ));
            } else {
                let key = normalize(label);
                if labels.insert(key, offset + 1).is_some() {
                    diagnostics.push(Diagnostic::new(
                        line.span,
                        format!("duplicate inline assembler label `{label}`"),
                    ));
                }
            }
        }
        instructions.push(ParsedInstruction {
            mnemonic: pending.mnemonic,
            operand: pending.operand,
            span: pending.span,
            offset,
            line_index: pending.line_index,
            opcode,
            mode: addressing,
        });
        offset += addressing_len(addressing);
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut items = Vec::new();
    let mut relocations = Vec::new();
    for instruction in &instructions {
        items.push(byte_item(instruction.opcode));
        if let Err(diagnostic) = emit_operand_items(
            instruction,
            &constants,
            &labels,
            &anonymous_labels,
            &mut items,
            &mut relocations,
        ) {
            diagnostics.push(diagnostic);
        }
    }

    let bytes = machine_item_template(&items);
    if mode == InlineAsmMode::Analyzed {
        for instruction in &instructions {
            if matches!(instruction.mnemonic.as_str(), "TXS" | "BRK" | "RTI" | "SED") {
                diagnostics.push(Diagnostic::new(
                    instruction.span,
                    format!(
                        "`{}` requires `ASM OPAQUE` because its machine-state/control effect is not safely modeled",
                        instruction.mnemonic
                    ),
                ));
            }
        }
        if !analyze_machine_state(&bytes, &inline_control_targets(&relocations))
            .stack_balanced_at_exits
        {
            diagnostics.push(Diagnostic::new(
                instructions.last().map_or(
                    Span::new(source_offset, source_offset + source.len()),
                    |instruction| instruction.span,
                ),
                "inline assembler paths must have balanced stack depth at every exit; use `ASM OPAQUE` only for deliberate non-standard control",
            ));
        }
    }

    if diagnostics.is_empty() {
        Ok(InlineAsmProgram {
            bytes,
            items,
            relocations,
            source: source.to_string(),
            mode,
        })
    } else {
        Err(diagnostics)
    }
}

fn source_lines(source: &str, source_offset: usize) -> Vec<SourceLine> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for segment in source.split_inclusive('\n') {
        let text = segment.strip_suffix('\n').unwrap_or(segment);
        lines.push(SourceLine {
            text: text.to_string(),
            span: Span::new(source_offset + offset, source_offset + offset + text.len()),
        });
        offset += segment.len();
    }
    if source.is_empty() {
        lines.push(SourceLine {
            text: String::new(),
            span: Span::new(source_offset, source_offset),
        });
    }
    lines
}

fn strip_asm_comments(source: &str, source_offset: usize) -> Result<String, Diagnostic> {
    let bytes = source.as_bytes();
    let mut output = bytes.to_vec();
    let mut index = 0usize;
    let mut in_char = false;
    while index < bytes.len() {
        if in_char {
            if bytes[index] == b'\'' {
                in_char = false;
            }
            index += 1;
            continue;
        }
        if bytes[index] == b'\'' {
            in_char = true;
            index += 1;
            continue;
        }
        if bytes[index] == b';'
            || (bytes[index] == b'/' && bytes.get(index + 1).copied() == Some(b'/'))
        {
            while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                output[index] = b' ';
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1).copied() == Some(b'*') {
            let start = index;
            output[index] = b' ';
            output[index + 1] = b' ';
            index += 2;
            let mut terminated = false;
            while index < bytes.len() {
                if bytes[index] == b'*' && bytes.get(index + 1).copied() == Some(b'/') {
                    output[index] = b' ';
                    output[index + 1] = b' ';
                    index += 2;
                    terminated = true;
                    break;
                }
                if !matches!(bytes[index], b'\n' | b'\r') {
                    output[index] = b' ';
                }
                index += 1;
            }
            if !terminated {
                return Err(Diagnostic::new(
                    Span::new(source_offset + start, source_offset + source.len()),
                    "unterminated inline assembler block comment",
                ));
            }
            continue;
        }
        index += 1;
    }
    Ok(String::from_utf8(output).expect("comment replacement keeps UTF-8 bytes"))
}

fn split_label(text: &str) -> Option<(String, &str)> {
    let colon = text.find(':')?;
    let raw_candidate = &text[..colon];
    let candidate = raw_candidate.trim();
    if candidate.is_empty()
        || raw_candidate.chars().any(char::is_whitespace)
        || candidate
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '@' || ch == '.'))
    {
        return None;
    }
    Some((candidate.to_string(), &text[colon + 1..]))
}

fn split_optional_label(text: &str) -> Option<(&str, &str)> {
    let split = text.find(char::is_whitespace).unwrap_or(text.len());
    let candidate = &text[..split];
    if !valid_name(candidate) {
        return None;
    }
    if mnemonic_exists(candidate) {
        return None;
    }
    let rest = text[split..].trim();
    if rest.is_empty() || mnemonic_exists(rest.split_whitespace().next().unwrap_or("")) {
        return Some((candidate, rest));
    }
    None
}

fn split_constant(text: &str) -> Option<(&str, &str)> {
    if let Some((left, right)) = text.split_once('=') {
        let name = left.trim();
        if valid_name(name) {
            return Some((name, right.trim()));
        }
    }
    let mut parts = text.splitn(3, char::is_whitespace);
    let name = parts.next()?;
    let directive = parts.next()?;
    let expression = parts.next()?;
    (valid_name(name) && directive.eq_ignore_ascii_case("EQU")).then_some((name, expression.trim()))
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_' | '.'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn parse_instruction_line(
    text: &str,
    span: Span,
    line_index: usize,
) -> Result<PendingInstruction, Diagnostic> {
    let split = text.find(char::is_whitespace).unwrap_or(text.len());
    let mnemonic_token = &text[..split];
    let raw_operand = text[split..].trim();
    let (operand_label, operand) = split_operand_label(raw_operand)
        .map_or((None, raw_operand), |(label, operand)| {
            (Some(label.to_string()), operand)
        });
    let (mnemonic, suffix) = if let Some((mnemonic, suffix)) = mnemonic_token.rsplit_once('.') {
        let suffix = match suffix.to_ascii_uppercase().as_str() {
            "Z" | "B" => SizeSuffix::ZeroPage,
            "A" | "W" => SizeSuffix::Absolute,
            _ => {
                return Err(Diagnostic::new(
                    span,
                    format!("unsupported MADS instruction suffix `.{suffix}`"),
                ));
            }
        };
        (mnemonic, suffix)
    } else {
        (mnemonic_token, SizeSuffix::Default)
    };
    if mnemonic.is_empty() || !mnemonic.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Err(Diagnostic::new(
            span,
            "expected a 6502 instruction mnemonic",
        ));
    }
    Ok(PendingInstruction {
        mnemonic: mnemonic.to_ascii_uppercase(),
        suffix,
        operand: operand.to_string(),
        operand_label,
        span,
        line_index,
    })
}

fn split_operand_label(operand: &str) -> Option<(&str, &str)> {
    let (candidate, operand) = operand.split_once(':')?;
    let candidate = candidate.trim();
    let operand = operand.trim();
    valid_name(candidate).then_some((candidate, operand))
}

fn select_instruction(
    instruction: &PendingInstruction,
    constants: &HashMap<String, i32>,
    span: Span,
) -> Result<(u8, AddressingMode), Diagnostic> {
    let candidate_modes = operand_modes(
        &instruction.mnemonic,
        instruction.suffix,
        &instruction.operand,
        constants,
        span,
    )?;
    for mode in candidate_modes {
        if let Some(opcode) = opcode_for(&instruction.mnemonic, mode) {
            return Ok((opcode, mode));
        }
    }
    Err(Diagnostic::new(
        span,
        format!(
            "instruction `{}` does not support operand `{}`",
            instruction.mnemonic, instruction.operand
        ),
    ))
}

fn opcode_for(mnemonic: &str, mode: AddressingMode) -> Option<u8> {
    (0u16..=255).find_map(|opcode| {
        let (candidate, candidate_mode, _) = decode_6502_opcode(opcode as u8)?;
        (candidate.eq_ignore_ascii_case(mnemonic) && candidate_mode == mode).then_some(opcode as u8)
    })
}

fn mnemonic_exists(mnemonic: &str) -> bool {
    let mnemonic = mnemonic
        .rsplit_once('.')
        .map_or(mnemonic, |(mnemonic, _)| mnemonic);
    (0u16..=255).any(|opcode| {
        decode_6502_opcode(opcode as u8)
            .is_some_and(|(candidate, _, _)| candidate.eq_ignore_ascii_case(mnemonic))
    })
}

fn operand_modes(
    mnemonic: &str,
    suffix: SizeSuffix,
    operand: &str,
    constants: &HashMap<String, i32>,
    span: Span,
) -> Result<Vec<AddressingMode>, Diagnostic> {
    let operand = operand.trim();
    if opcode_for(mnemonic, AddressingMode::Relative).is_some() {
        if operand.is_empty() {
            return Err(Diagnostic::new(
                span,
                "branch instruction requires a target",
            ));
        }
        return Ok(vec![AddressingMode::Relative]);
    }
    if operand.is_empty() {
        return Ok(vec![AddressingMode::Implied]);
    }
    if operand.eq_ignore_ascii_case("A") {
        return Ok(vec![AddressingMode::Accumulator]);
    }
    if let Some(expression) = operand.strip_prefix('#') {
        parse_expression(expression.trim(), constants, span)?;
        return Ok(vec![AddressingMode::Immediate]);
    }

    let compact = compact_operand(operand);
    if compact.starts_with('(') {
        if let Some(inner) = strip_ascii_suffix(&compact, "),Y") {
            let inner = inner
                .strip_prefix('(')
                .ok_or_else(|| Diagnostic::new(span, "invalid indirect operand"))?;
            parse_expression(inner, constants, span)?;
            return Ok(vec![AddressingMode::IndirectIndexedY]);
        }
        if let Some(inner) = strip_ascii_suffix(&compact, ",X)") {
            let inner = inner
                .strip_prefix('(')
                .ok_or_else(|| Diagnostic::new(span, "invalid indirect operand"))?;
            parse_expression(inner, constants, span)?;
            return Ok(vec![AddressingMode::IndexedIndirectX]);
        }
        if compact.ends_with(')') {
            parse_expression(&compact[1..compact.len() - 1], constants, span)?;
            return Ok(vec![AddressingMode::Indirect]);
        }
        return Err(Diagnostic::new(span, "invalid indirect operand syntax"));
    }

    let (expression, index) = if let Some(expression) = strip_ascii_suffix(&compact, ",X") {
        (expression, Some('X'))
    } else if let Some(expression) = strip_ascii_suffix(&compact, ",Y") {
        (expression, Some('Y'))
    } else {
        (compact.as_str(), None)
    };
    let expression = parse_expression(expression, constants, span)?;
    let numeric = resolve_numeric_expr(&expression, constants);
    let byte_sized = match suffix {
        SizeSuffix::ZeroPage => true,
        SizeSuffix::Absolute => false,
        SizeSuffix::Default => numeric.is_some_and(|value| (0..=255).contains(&value)),
    };

    Ok(match (index, byte_sized) {
        (None, true) => vec![AddressingMode::ZeroPage, AddressingMode::Absolute],
        (None, false) => vec![AddressingMode::Absolute],
        (Some('X'), true) => vec![AddressingMode::ZeroPageX, AddressingMode::AbsoluteX],
        (Some('X'), false) => vec![AddressingMode::AbsoluteX],
        (Some('Y'), true) => vec![AddressingMode::ZeroPageY, AddressingMode::AbsoluteY],
        (Some('Y'), false) => vec![AddressingMode::AbsoluteY],
        _ => unreachable!(),
    })
}

fn compact_operand(operand: &str) -> String {
    operand.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn strip_ascii_suffix<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    if text.len() >= suffix.len() && text[text.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    {
        Some(&text[..text.len() - suffix.len()])
    } else {
        None
    }
}

fn parse_expression(
    text: &str,
    constants: &HashMap<String, i32>,
    span: Span,
) -> Result<AsmExpr, Diagnostic> {
    let mut text = text.trim();
    let selector = if let Some(rest) = text.strip_prefix('<') {
        text = rest.trim();
        Some(AddressByteSelector::Low)
    } else if let Some(rest) = text.strip_prefix('>') {
        text = rest.trim();
        Some(AddressByteSelector::High)
    } else {
        None
    };
    if text.is_empty() {
        return Err(Diagnostic::new(span, "expected assembler expression"));
    }

    if !matches!(text, "@+" | "@-") && !text.starts_with([':', '*']) {
        match NumericExprParser::new(text, constants).parse() {
            Ok(Some(value)) => {
                return Ok(AsmExpr {
                    selector,
                    atom: ExprAtom::Number(value),
                    addend: 0,
                });
            }
            Ok(None) => {}
            Err(message) => return Err(Diagnostic::new(span, message)),
        }
    }

    let (atom_text, addend) = if matches!(text, "@+" | "@-") {
        (text, 0)
    } else {
        split_addend(text, span)?
    };
    let atom = if atom_text == "*" {
        ExprAtom::Current
    } else if atom_text == "@+" {
        ExprAtom::AnonymousForward
    } else if atom_text == "@-" {
        ExprAtom::AnonymousBackward
    } else if let Some(value) = parse_number(atom_text) {
        ExprAtom::Number(value)
    } else if let Some(value) = constants.get(&normalize(atom_text)) {
        ExprAtom::Number(*value)
    } else {
        let (name, external) = atom_text
            .strip_prefix(':')
            .map_or((atom_text, false), |name| (name, true));
        if !valid_name(name) {
            return Err(Diagnostic::new(
                span,
                format!("unsupported assembler expression `{text}`"),
            ));
        }
        if external {
            ExprAtom::ExternalName(name.to_string())
        } else {
            ExprAtom::Name(name.to_string())
        }
    };

    Ok(AsmExpr {
        selector,
        atom,
        addend,
    })
}

struct NumericExprParser<'a> {
    text: &'a str,
    pos: usize,
    constants: &'a HashMap<String, i32>,
    unresolved: bool,
}

impl<'a> NumericExprParser<'a> {
    fn new(text: &'a str, constants: &'a HashMap<String, i32>) -> Self {
        Self {
            text,
            pos: 0,
            constants,
            unresolved: false,
        }
    }

    fn parse(mut self) -> Result<Option<i32>, String> {
        let value = self.parse_or()?;
        self.skip_space();
        if self.pos != self.text.len() {
            return Ok(None);
        }
        Ok((!self.unresolved).then_some(value))
    }

    fn parse_or(&mut self) -> Result<i32, String> {
        let mut value = self.parse_xor()?;
        while self.consume("|") {
            value |= self.parse_xor()?;
        }
        Ok(value)
    }

    fn parse_xor(&mut self) -> Result<i32, String> {
        let mut value = self.parse_and()?;
        while self.consume("^") {
            value ^= self.parse_and()?;
        }
        Ok(value)
    }

    fn parse_and(&mut self) -> Result<i32, String> {
        let mut value = self.parse_shift()?;
        while self.consume("&") {
            value &= self.parse_shift()?;
        }
        Ok(value)
    }

    fn parse_shift(&mut self) -> Result<i32, String> {
        let mut value = self.parse_add()?;
        loop {
            if self.consume("<<") {
                let shift = self.parse_add()?;
                value = value
                    .checked_shl(u32::try_from(shift).map_err(|_| {
                        "inline assembler shift count must be non-negative".to_string()
                    })?)
                    .ok_or_else(|| "inline assembler shift is out of range".to_string())?;
            } else if self.consume(">>") {
                let shift = self.parse_add()?;
                value = value
                    .checked_shr(u32::try_from(shift).map_err(|_| {
                        "inline assembler shift count must be non-negative".to_string()
                    })?)
                    .ok_or_else(|| "inline assembler shift is out of range".to_string())?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_add(&mut self) -> Result<i32, String> {
        let mut value = self.parse_mul()?;
        loop {
            if self.consume("+") {
                value = value
                    .checked_add(self.parse_mul()?)
                    .ok_or_else(|| "inline assembler expression overflow".to_string())?;
            } else if self.consume("-") {
                value = value
                    .checked_sub(self.parse_mul()?)
                    .ok_or_else(|| "inline assembler expression overflow".to_string())?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_mul(&mut self) -> Result<i32, String> {
        let mut value = self.parse_unary()?;
        loop {
            if self.consume("*") {
                value = value
                    .checked_mul(self.parse_unary()?)
                    .ok_or_else(|| "inline assembler expression overflow".to_string())?;
            } else if self.consume("/") {
                let divisor = self.parse_unary()?;
                value = value
                    .checked_div(divisor)
                    .ok_or_else(|| "inline assembler division by zero or overflow".to_string())?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_unary(&mut self) -> Result<i32, String> {
        if self.consume("+") {
            return self.parse_unary();
        }
        if self.consume("-") {
            return self
                .parse_unary()?
                .checked_neg()
                .ok_or_else(|| "inline assembler expression overflow".to_string());
        }
        if self.consume("~") {
            return Ok(!self.parse_unary()?);
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<i32, String> {
        self.skip_space();
        if self.consume("(") {
            let value = self.parse_or()?;
            if !self.consume(")") {
                return Err("missing `)` in inline assembler expression".to_string());
            }
            return Ok(value);
        }
        let start = self.pos;
        if self.peek() == Some('$') {
            self.pos += 1;
            self.take_while(|ch| ch.is_ascii_hexdigit());
        } else if self.peek() == Some('%') {
            self.pos += 1;
            self.take_while(|ch| matches!(ch, '0' | '1'));
        } else if self.peek() == Some('\'') {
            self.pos += 1;
            while self.peek().is_some_and(|ch| ch != '\'') {
                self.pos += self.peek().map(char::len_utf8).unwrap_or(1);
            }
            if self.peek() == Some('\'') {
                self.pos += 1;
            }
        } else if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.take_while(|ch| ch.is_ascii_digit());
        } else if self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | '.'))
        {
            self.take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.'));
            let name = &self.text[start..self.pos];
            if let Some(value) = self.constants.get(&normalize(name)) {
                return Ok(*value);
            }
            self.unresolved = true;
            return Ok(0);
        } else {
            return Err("expected a numeric inline assembler expression".to_string());
        }
        parse_number(&self.text[start..self.pos]).ok_or_else(|| {
            format!(
                "invalid inline assembler number `{}`",
                &self.text[start..self.pos]
            )
        })
    }

    fn consume(&mut self, token: &str) -> bool {
        self.skip_space();
        if self.text[self.pos..].starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn take_while(&mut self, predicate: impl Fn(char) -> bool) {
        while let Some(ch) = self.peek() {
            if !predicate(ch) {
                break;
            }
            self.pos += ch.len_utf8();
        }
    }

    fn skip_space(&mut self) {
        self.take_while(char::is_whitespace);
    }

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }
}

fn split_addend(text: &str, span: Span) -> Result<(&str, i32), Diagnostic> {
    let mut split = None;
    for (index, ch) in text.char_indices().skip(1) {
        if ch == '+' || ch == '-' {
            split = Some((index, ch));
        }
    }
    let Some((index, sign)) = split else {
        return Ok((text.trim(), 0));
    };
    let atom = text[..index].trim();
    let value_text = text[index + 1..].trim();
    let Some(value) = parse_number(value_text) else {
        return Err(Diagnostic::new(
            span,
            "relocatable expressions support only a numeric addend",
        ));
    };
    Ok((atom, if sign == '-' { -value } else { value }))
}

fn parse_number(text: &str) -> Option<i32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('$') {
        i32::from_str_radix(hex, 16).ok()
    } else if let Some(binary) = text.strip_prefix('%') {
        i32::from_str_radix(binary, 2).ok()
    } else if text.starts_with('\'') && text.ends_with('\'') && text.len() >= 3 {
        let body = &text[1..text.len() - 1];
        if let Some(escape) = body
            .strip_prefix("\\{")
            .and_then(|body| body.strip_suffix('}'))
        {
            let decoded = decode_atascii_escape(escape).ok()?;
            return match decoded.as_slice() {
                [ch] => crate::source::source_char_byte(*ch).map(i32::from),
                _ => None,
            };
        }
        let mut chars = body.chars();
        let ch = chars.next()?;
        chars
            .next()
            .is_none()
            .then(|| crate::source::source_char_byte(ch).map(i32::from))
            .flatten()
    } else {
        text.parse::<i32>().ok()
    }
}

fn resolve_numeric_expr(expr: &AsmExpr, constants: &HashMap<String, i32>) -> Option<i32> {
    let value = match &expr.atom {
        ExprAtom::Number(value) => *value,
        ExprAtom::Current => return None,
        ExprAtom::Name(name) => *constants.get(&normalize(name))?,
        ExprAtom::ExternalName(_) => return None,
        ExprAtom::AnonymousForward | ExprAtom::AnonymousBackward => return None,
    }
    .checked_add(expr.addend)?;
    Some(match expr.selector {
        Some(AddressByteSelector::Low) => value & 0xff,
        Some(AddressByteSelector::High) => (value >> 8) & 0xff,
        None => value,
    })
}

fn emit_operand_items(
    instruction: &ParsedInstruction,
    constants: &HashMap<String, i32>,
    labels: &HashMap<String, usize>,
    anonymous_labels: &[(usize, usize)],
    items: &mut Vec<MachineItem>,
    relocations: &mut Vec<InlineAsmRelocation>,
) -> Result<(), Diagnostic> {
    if matches!(
        instruction.mode,
        AddressingMode::Implied | AddressingMode::Accumulator
    ) {
        return Ok(());
    }

    let expression_text = operand_expression(&instruction.operand, instruction.mode)
        .ok_or_else(|| Diagnostic::new(instruction.span, "invalid assembler operand"))?;
    let expr = parse_expression(expression_text, constants, instruction.span)?;

    if instruction.mode == AddressingMode::Relative {
        let target = resolve_local_target(&expr, instruction, labels, anonymous_labels, constants)?;
        let after = instruction.offset as i32 + 2;
        let displacement = target - after;
        if !(-128..=127).contains(&displacement) {
            return Err(Diagnostic::new(
                instruction.span,
                format!("branch target is out of range ({displacement} bytes)"),
            ));
        }
        items.push(byte_item(displacement as i8 as u8));
        return Ok(());
    }

    let width = addressing_len(instruction.mode) - 1;
    if let Some(value) = resolve_numeric_expr(&expr, constants) {
        emit_numeric_operand(value, width, instruction.span, items)?;
        if instruction.mode != AddressingMode::Immediate {
            let target = u16::try_from(value).map_err(|_| {
                Diagnostic::new(
                    instruction.span,
                    format!("assembler address `{value}` does not fit in 16 bits"),
                )
            })?;
            relocations.push(InlineAsmRelocation {
                offset: (instruction.offset + 1) as u16,
                kind: if width == 1 {
                    InlineAsmRelocationKind::Low8
                } else {
                    InlineAsmRelocationKind::Absolute16
                },
                target: InlineAsmRelocationTarget::Absolute(target),
                addend: 0,
                requires_zero_page: matches!(
                    instruction.mode,
                    AddressingMode::ZeroPage
                        | AddressingMode::ZeroPageX
                        | AddressingMode::ZeroPageY
                        | AddressingMode::IndexedIndirectX
                        | AddressingMode::IndirectIndexedY
                ),
                symbol_use: symbol_use(instruction),
                span: instruction.span,
            });
        }
        return Ok(());
    }

    let name = match &expr.atom {
        ExprAtom::Name(name) | ExprAtom::ExternalName(name) => name,
        _ => {
            if matches!(
                expr.atom,
                ExprAtom::AnonymousForward | ExprAtom::AnonymousBackward | ExprAtom::Current
            ) {
                let target =
                    resolve_local_target(&expr, instruction, labels, anonymous_labels, constants)?;
                return emit_inline_target(
                    target,
                    0,
                    width,
                    instruction.offset + 1,
                    instruction.span,
                    items,
                    relocations,
                    instruction,
                    expr.selector,
                );
            }
            return Err(Diagnostic::new(
                instruction.span,
                "unresolved assembler expression",
            ));
        }
    };

    if matches!(expr.atom, ExprAtom::Name(_))
        && let Some(target) = labels.get(&normalize(name))
    {
        return emit_inline_target(
            *target as i32,
            expr.addend,
            width,
            instruction.offset + 1,
            instruction.span,
            items,
            relocations,
            instruction,
            expr.selector,
        );
    }

    let exact_byte_constant =
        width == 1 && expr.selector.is_none() && instruction.mode == AddressingMode::Immediate;
    let selector = if width == 1 {
        Some(expr.selector.unwrap_or(AddressByteSelector::Low))
    } else {
        if expr.selector.is_some() {
            return Err(Diagnostic::new(
                instruction.span,
                "low/high-byte selector cannot be used with a 16-bit operand",
            ));
        }
        None
    };
    items.push(MachineItem::AddressExpr(MachineAddressExpr {
        selector,
        explicit_address: true,
        atom: MachineAddressAtom::Name(qualified_asm_name(name)),
        offset: expr.addend,
        text: expression_text.to_string(),
    }));
    relocations.push(InlineAsmRelocation {
        offset: (instruction.offset + 1) as u16,
        kind: if exact_byte_constant {
            InlineAsmRelocationKind::Byte8
        } else {
            match selector {
                Some(AddressByteSelector::Low) => InlineAsmRelocationKind::Low8,
                Some(AddressByteSelector::High) => InlineAsmRelocationKind::High8,
                None => InlineAsmRelocationKind::Absolute16,
            }
        },
        target: InlineAsmRelocationTarget::Symbol(name.clone()),
        addend: expr.addend,
        requires_zero_page: matches!(
            instruction.mode,
            AddressingMode::ZeroPage
                | AddressingMode::ZeroPageX
                | AddressingMode::ZeroPageY
                | AddressingMode::IndexedIndirectX
                | AddressingMode::IndirectIndexedY
        ),
        symbol_use: if exact_byte_constant {
            InlineAsmSymbolUse::Constant
        } else {
            symbol_use(instruction)
        },
        span: instruction.span,
    });
    Ok(())
}

fn qualified_asm_name(name: &str) -> QualifiedName {
    QualifiedName::new(name.split('.').map(str::to_string).collect())
}

fn operand_expression(operand: &str, mode: AddressingMode) -> Option<&str> {
    let operand = operand.trim();
    match mode {
        AddressingMode::Immediate => operand.strip_prefix('#').map(str::trim),
        AddressingMode::IndexedIndirectX => {
            let inner = operand.strip_prefix('(')?.strip_suffix(')')?;
            let comma = inner.rfind(',')?;
            inner[comma + 1..]
                .trim()
                .eq_ignore_ascii_case("X")
                .then_some(inner[..comma].trim())
        }
        AddressingMode::IndirectIndexedY => {
            let close = operand.rfind(')')?;
            operand[close + 1..]
                .trim()
                .strip_prefix(',')?
                .trim()
                .eq_ignore_ascii_case("Y")
                .then(|| operand[..close].trim().strip_prefix('(').map(str::trim))
                .flatten()
        }
        AddressingMode::Indirect => operand
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .map(str::trim),
        AddressingMode::ZeroPageX | AddressingMode::AbsoluteX => strip_index_suffix(operand, 'X'),
        AddressingMode::ZeroPageY | AddressingMode::AbsoluteY => strip_index_suffix(operand, 'Y'),
        AddressingMode::ZeroPage | AddressingMode::Absolute | AddressingMode::Relative => {
            Some(operand)
        }
        AddressingMode::Implied | AddressingMode::Accumulator => None,
    }
}

fn strip_index_suffix(operand: &str, register: char) -> Option<&str> {
    let comma = operand.rfind(',')?;
    operand[comma + 1..]
        .trim()
        .eq_ignore_ascii_case(&register.to_string())
        .then_some(operand[..comma].trim())
}

fn resolve_local_target(
    expr: &AsmExpr,
    instruction: &ParsedInstruction,
    labels: &HashMap<String, usize>,
    anonymous_labels: &[(usize, usize)],
    constants: &HashMap<String, i32>,
) -> Result<i32, Diagnostic> {
    let target = match &expr.atom {
        ExprAtom::Name(name) => labels
            .get(&normalize(name))
            .copied()
            .map(|value| value as i32),
        ExprAtom::ExternalName(_) => None,
        ExprAtom::AnonymousForward => anonymous_labels
            .iter()
            .find(|(line, _)| *line > instruction.line_index)
            .map(|(_, offset)| *offset as i32),
        ExprAtom::AnonymousBackward => anonymous_labels
            .iter()
            .rev()
            .find(|(line, _)| *line < instruction.line_index)
            .map(|(_, offset)| *offset as i32),
        ExprAtom::Current => Some(instruction.offset as i32),
        _ => resolve_numeric_expr(expr, constants),
    };
    target
        .and_then(|value| value.checked_add(expr.addend))
        .ok_or_else(|| {
            Diagnostic::new(
                instruction.span,
                "branches may target only labels inside the same ASM block",
            )
        })
}

fn emit_numeric_operand(
    value: i32,
    width: usize,
    span: Span,
    items: &mut Vec<MachineItem>,
) -> Result<(), Diagnostic> {
    match width {
        1 if (0..=255).contains(&value) => items.push(byte_item(value as u8)),
        2 if (0..=65535).contains(&value) => {
            items.push(byte_item(value as u8));
            items.push(byte_item((value >> 8) as u8));
        }
        1 => {
            return Err(Diagnostic::new(
                span,
                format!("assembler operand `{value}` does not fit in one byte"),
            ));
        }
        2 => {
            return Err(Diagnostic::new(
                span,
                format!("assembler operand `{value}` does not fit in two bytes"),
            ));
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn emit_inline_target(
    target: i32,
    addend: i32,
    width: usize,
    operand_offset: usize,
    span: Span,
    items: &mut Vec<MachineItem>,
    relocations: &mut Vec<InlineAsmRelocation>,
    instruction: &ParsedInstruction,
    selector: Option<AddressByteSelector>,
) -> Result<(), Diagnostic> {
    if width == 1 && selector.is_none() {
        return Err(Diagnostic::new(
            span,
            "an inline label byte requires an explicit low/high selector",
        ));
    }
    if !matches!(width, 1 | 2) {
        unreachable!();
    }
    items.push(MachineItem::AddressExpr(MachineAddressExpr {
        selector,
        explicit_address: true,
        atom: MachineAddressAtom::Current,
        offset: target + addend - operand_offset as i32,
        text: "*".to_string(),
    }));
    let inline_offset = target
        .checked_add(addend)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| Diagnostic::new(span, "inline label address is outside the ASM block"))?;
    relocations.push(InlineAsmRelocation {
        offset: operand_offset as u16,
        kind: match selector {
            Some(AddressByteSelector::Low) => InlineAsmRelocationKind::Low8,
            Some(AddressByteSelector::High) => InlineAsmRelocationKind::High8,
            None => InlineAsmRelocationKind::Absolute16,
        },
        target: InlineAsmRelocationTarget::InlineOffset(inline_offset),
        addend: 0,
        requires_zero_page: false,
        symbol_use: symbol_use(instruction),
        span,
    });
    Ok(())
}

fn symbol_use(instruction: &ParsedInstruction) -> InlineAsmSymbolUse {
    if instruction.mnemonic == "JSR" {
        return InlineAsmSymbolUse::Call;
    }
    if instruction.mnemonic == "JMP" {
        return if instruction.mode == AddressingMode::Indirect {
            InlineAsmSymbolUse::PointerRead
        } else {
            InlineAsmSymbolUse::Control
        };
    }
    if instruction.mode == AddressingMode::Immediate {
        return InlineAsmSymbolUse::Address;
    }
    if matches!(
        instruction.mode,
        AddressingMode::IndexedIndirectX | AddressingMode::IndirectIndexedY
    ) {
        return InlineAsmSymbolUse::PointerRead;
    }
    let use_kind = match instruction.mnemonic.as_str() {
        "STA" | "STX" | "STY" => InlineAsmSymbolUse::Write,
        "ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC" => InlineAsmSymbolUse::ReadWrite,
        _ => InlineAsmSymbolUse::Read,
    };
    if matches!(
        instruction.mode,
        AddressingMode::ZeroPageX
            | AddressingMode::ZeroPageY
            | AddressingMode::AbsoluteX
            | AddressingMode::AbsoluteY
    ) {
        return match use_kind {
            InlineAsmSymbolUse::Read => InlineAsmSymbolUse::IndexedRead,
            InlineAsmSymbolUse::Write => InlineAsmSymbolUse::IndexedWrite,
            InlineAsmSymbolUse::ReadWrite => InlineAsmSymbolUse::IndexedReadWrite,
            _ => unreachable!(),
        };
    }
    use_kind
}

pub(crate) fn ends_in_terminal_instruction(bytes: &[u8]) -> bool {
    let mut offset = 0usize;
    let mut last_mnemonic = None;
    while offset < bytes.len() {
        let Some((mnemonic, _, len)) = decode_6502_opcode(bytes[offset]) else {
            return false;
        };
        if offset + len > bytes.len() {
            return false;
        }
        last_mnemonic = Some(mnemonic);
        offset += len;
    }
    matches!(last_mnemonic, Some("JMP" | "RTS" | "RTI" | "BRK"))
}

fn machine_item_template(items: &[MachineItem]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in items {
        match item {
            MachineItem::Number(number) => bytes.push(number.value.unwrap_or(0) as u8),
            MachineItem::AddressExpr(expr) if expr.selector.is_none() => {
                bytes.extend([0, 0]);
            }
            MachineItem::AddressExpr(_) | MachineItem::AddressByte { .. } => bytes.push(0),
            MachineItem::CharLiteral(ch) => bytes.push(*ch as u8),
            MachineItem::StringLiteral(text) => {
                bytes.extend(text.chars().map(|ch| ch as u8));
            }
            MachineItem::Name(_) | MachineItem::Raw(_) => {}
        }
    }
    bytes
}

fn byte_item(value: u8) -> MachineItem {
    MachineItem::Number(NumberLiteral {
        text: format!("${value:02X}"),
        kind: NumberKind::Byte,
        value: Some(u16::from(value)),
    })
}

fn addressing_len(mode: AddressingMode) -> usize {
    match mode {
        AddressingMode::Implied | AddressingMode::Accumulator => 1,
        AddressingMode::Immediate
        | AddressingMode::ZeroPage
        | AddressingMode::ZeroPageX
        | AddressingMode::ZeroPageY
        | AddressingMode::IndexedIndirectX
        | AddressingMode::IndirectIndexedY
        | AddressingMode::Relative => 2,
        AddressingMode::Absolute
        | AddressingMode::AbsoluteX
        | AddressingMode::AbsoluteY
        | AddressingMode::Indirect => 3,
    }
}

fn normalize(name: &str) -> String {
    name.to_ascii_uppercase()
}

/// Summarize incoming machine-state uses and outgoing clobbers for an already
/// encoded inline block. Relative branches are followed inside the block;
/// absolute jumps are terminal here because their relocations are resolved by
/// MIR emission.
pub(crate) fn analyze_machine_state(
    bytes: &[u8],
    local_control_targets: &[(u16, u16)],
) -> InlineAsmMachineState {
    #[derive(Clone, Copy, Default)]
    struct InstructionEffects {
        reads: InlineAsmRegisterSet,
        writes: InlineAsmRegisterSet,
        stack_delta: Option<i8>,
    }

    let mut offsets = Vec::new();
    let mut instructions = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let Some((mnemonic, mode, len)) = decode_6502_opcode(bytes[offset]) else {
            return InlineAsmMachineState {
                reads: all_inline_registers(),
                clobbers: all_inline_registers(),
                stack_depth_delta: None,
                stack_balanced_at_exits: false,
            };
        };
        if offset + len > bytes.len() {
            return InlineAsmMachineState {
                reads: all_inline_registers(),
                clobbers: all_inline_registers(),
                stack_depth_delta: None,
                stack_balanced_at_exits: false,
            };
        }
        offsets.push(offset);
        instructions.push((mnemonic, mode, len, instruction_effects(mnemonic, mode)));
        offset += len;
    }

    let offset_to_index = offsets
        .iter()
        .enumerate()
        .map(|(index, offset)| (*offset, index))
        .collect::<HashMap<_, _>>();
    let local_control_targets = local_control_targets
        .iter()
        .copied()
        .collect::<HashMap<_, _>>();
    let mut successors = vec![Vec::<usize>::new(); instructions.len()];
    let mut exits = vec![false; instructions.len()];
    for (index, (mnemonic, mode, len, _)) in instructions.iter().enumerate() {
        let next = index + 1;
        if *mode == AddressingMode::Relative {
            if next < instructions.len() {
                successors[index].push(next);
            } else {
                exits[index] = true;
            }
            let displacement = bytes[offsets[index] + 1] as i8 as isize;
            let target = offsets[index] as isize + *len as isize + displacement;
            if let Ok(target) = usize::try_from(target)
                && let Some(target_index) = offset_to_index.get(&target)
            {
                successors[index].push(*target_index);
            }
        } else if *mnemonic == "JMP" {
            let operand_offset = u16::try_from(offsets[index] + 1).ok();
            let target = operand_offset
                .and_then(|offset| local_control_targets.get(&offset))
                .and_then(|offset| offset_to_index.get(&usize::from(*offset)));
            if let Some(target) = target {
                successors[index].push(*target);
            } else {
                exits[index] = true;
            }
        } else if matches!(*mnemonic, "RTS" | "RTI" | "BRK") {
            // No fall-through edge. Local absolute JMP targets use relocation
            // records supplied above.
            exits[index] = true;
        } else if next < instructions.len() {
            successors[index].push(next);
        } else {
            exits[index] = true;
        }
    }

    let mut predecessors = vec![Vec::<usize>::new(); instructions.len()];
    for (index, targets) in successors.iter().enumerate() {
        for target in targets {
            predecessors[*target].push(index);
        }
    }

    let mut incoming_defs = vec![None::<InlineAsmRegisterSet>; instructions.len()];
    let mut outgoing_defs = vec![InlineAsmRegisterSet::default(); instructions.len()];
    let mut incoming_depth = vec![None::<i16>; instructions.len()];
    if !instructions.is_empty() {
        incoming_defs[0] = Some(InlineAsmRegisterSet::default());
        incoming_depth[0] = Some(0);
    }

    let mut changed = true;
    while changed {
        changed = false;
        for index in 0..instructions.len() {
            let mut defs = if index == 0 {
                Some(InlineAsmRegisterSet::default())
            } else {
                None
            };
            for predecessor in &predecessors[index] {
                defs = Some(match defs {
                    Some(current) => {
                        intersect_inline_registers(current, outgoing_defs[*predecessor])
                    }
                    None => outgoing_defs[*predecessor],
                });
            }
            if defs != incoming_defs[index] {
                incoming_defs[index] = defs;
                changed = true;
            }
            let Some(defs) = defs else {
                continue;
            };
            let mut out = defs;
            merge_inline_registers(&mut out, instructions[index].3.writes);
            if out != outgoing_defs[index] {
                outgoing_defs[index] = out;
                changed = true;
            }

            let depth = if index == 0 {
                Some(0)
            } else {
                let mut depths = predecessors[index].iter().filter_map(|predecessor| {
                    incoming_depth[*predecessor].and_then(|depth| {
                        instructions[*predecessor]
                            .3
                            .stack_delta
                            .map(|delta| depth + i16::from(delta))
                    })
                });
                let first = depths.next();
                if first.is_some() && depths.all(|candidate| Some(candidate) == first) {
                    first
                } else {
                    None
                }
            };
            if depth != incoming_depth[index] {
                incoming_depth[index] = depth;
                changed = true;
            }
        }
    }

    let mut reads = InlineAsmRegisterSet::default();
    let mut clobbers = InlineAsmRegisterSet::default();
    for (index, (_, _, _, effects)) in instructions.iter().enumerate() {
        let Some(defs) = incoming_defs[index] else {
            continue;
        };
        merge_inline_registers(&mut reads, subtract_inline_registers(effects.reads, defs));
        merge_inline_registers(&mut clobbers, effects.writes);
    }

    let mut exit_depth = None;
    let mut compatible_exit_depths = true;
    let mut stack_balanced_at_exits = true;
    for index in 0..instructions.len() {
        if !exits[index] || incoming_defs[index].is_none() {
            continue;
        }
        let terminal = matches!(instructions[index].0, "JMP" | "RTS" | "RTI" | "BRK");
        let depth = incoming_depth[index].and_then(|depth| {
            if terminal {
                Some(depth)
            } else {
                instructions[index]
                    .3
                    .stack_delta
                    .map(|delta| depth + i16::from(delta))
            }
        });
        stack_balanced_at_exits &= depth == Some(0);
        match (exit_depth, depth) {
            (None, Some(depth)) => exit_depth = Some(depth),
            (Some(expected), Some(depth)) if expected == depth => {}
            _ => compatible_exit_depths = false,
        }
    }
    let stack_depth_delta = if compatible_exit_depths {
        exit_depth.and_then(|depth| i8::try_from(depth).ok())
    } else {
        None
    };

    let summary = InlineAsmMachineState {
        reads,
        clobbers,
        stack_depth_delta,
        stack_balanced_at_exits,
    };

    fn instruction_effects(mnemonic: &str, mode: AddressingMode) -> InstructionEffects {
        let mut effects = InstructionEffects {
            stack_delta: Some(0),
            ..InstructionEffects::default()
        };
        if matches!(
            mode,
            AddressingMode::ZeroPageX
                | AddressingMode::AbsoluteX
                | AddressingMode::IndexedIndirectX
        ) {
            effects.reads.x = true;
        }
        if matches!(
            mode,
            AddressingMode::ZeroPageY
                | AddressingMode::AbsoluteY
                | AddressingMode::IndirectIndexedY
        ) {
            effects.reads.y = true;
        }

        match mnemonic {
            "ADC" | "AND" | "CMP" | "EOR" | "ORA" | "SBC" | "STA" | "PHA" | "TAX" | "TAY" => {
                effects.reads.a = true
            }
            "ASL" | "LSR" | "ROL" | "ROR" if mode == AddressingMode::Accumulator => {
                effects.reads.a = true
            }
            _ => {}
        }
        match mnemonic {
            "ADC" | "AND" | "EOR" | "LDA" | "ORA" | "PLA" | "SBC" | "TXA" | "TYA" => {
                effects.writes.a = true
            }
            "ASL" | "LSR" | "ROL" | "ROR" if mode == AddressingMode::Accumulator => {
                effects.writes.a = true
            }
            _ => {}
        }
        if matches!(mnemonic, "CPX" | "DEX" | "INX" | "STX" | "TXA" | "TXS") {
            effects.reads.x = true;
        }
        if matches!(mnemonic, "DEX" | "INX" | "LDX" | "TAX" | "TSX") {
            effects.writes.x = true;
        }
        if matches!(mnemonic, "CPY" | "DEY" | "INY" | "STY" | "TYA") {
            effects.reads.y = true;
        }
        if matches!(mnemonic, "DEY" | "INY" | "LDY" | "TAY") {
            effects.writes.y = true;
        }

        if matches!(
            mnemonic,
            "ADC"
                | "SBC"
                | "ROL"
                | "ROR"
                | "BCC"
                | "BCS"
                | "BEQ"
                | "BMI"
                | "BNE"
                | "BPL"
                | "BVC"
                | "BVS"
                | "PHP"
        ) {
            effects.reads.flags = true;
        }
        if matches!(
            mnemonic,
            "ADC"
                | "AND"
                | "ASL"
                | "BIT"
                | "CLC"
                | "CLD"
                | "CLI"
                | "CLV"
                | "CMP"
                | "CPX"
                | "CPY"
                | "DEC"
                | "DEX"
                | "DEY"
                | "EOR"
                | "INC"
                | "INX"
                | "INY"
                | "LDA"
                | "LDX"
                | "LDY"
                | "LSR"
                | "ORA"
                | "PLA"
                | "PLP"
                | "ROL"
                | "ROR"
                | "RTI"
                | "SBC"
                | "SEC"
                | "SED"
                | "SEI"
                | "TAX"
                | "TAY"
                | "TSX"
                | "TXA"
                | "TYA"
        ) {
            effects.writes.flags = true;
        }

        match mnemonic {
            "PHA" | "PHP" => {
                effects.reads.sp = true;
                effects.writes.sp = true;
                effects.stack_delta = Some(-1);
            }
            "PLA" | "PLP" => {
                effects.reads.sp = true;
                effects.writes.sp = true;
                effects.stack_delta = Some(1);
            }
            "JSR" => {
                effects.reads.sp = true;
                effects.reads.a = true;
                effects.reads.x = true;
                effects.reads.y = true;
                effects.writes.a = true;
                effects.writes.x = true;
                effects.writes.y = true;
                effects.writes.flags = true;
            }
            "RTS" | "RTI" | "BRK" => {
                effects.reads.sp = true;
                effects.stack_delta = None;
            }
            "TSX" => effects.reads.sp = true,
            "TXS" => {
                effects.writes.sp = true;
                effects.stack_delta = None;
            }
            _ => {}
        }
        effects
    }

    summary
}

fn all_inline_registers() -> InlineAsmRegisterSet {
    InlineAsmRegisterSet {
        a: true,
        x: true,
        y: true,
        flags: true,
        sp: true,
    }
}

fn inline_control_targets(relocations: &[InlineAsmRelocation]) -> Vec<(u16, u16)> {
    relocations
        .iter()
        .filter_map(|relocation| {
            let InlineAsmRelocationTarget::InlineOffset(target) = &relocation.target else {
                return None;
            };
            (relocation.symbol_use == InlineAsmSymbolUse::Control)
                .then_some((relocation.offset, *target))
        })
        .collect()
}

fn merge_inline_registers(into: &mut InlineAsmRegisterSet, other: InlineAsmRegisterSet) {
    into.a |= other.a;
    into.x |= other.x;
    into.y |= other.y;
    into.flags |= other.flags;
    into.sp |= other.sp;
}

fn intersect_inline_registers(
    left: InlineAsmRegisterSet,
    right: InlineAsmRegisterSet,
) -> InlineAsmRegisterSet {
    InlineAsmRegisterSet {
        a: left.a && right.a,
        x: left.x && right.x,
        y: left.y && right.y,
        flags: left.flags && right.flags,
        sp: left.sp && right.sp,
    }
}

fn subtract_inline_registers(
    left: InlineAsmRegisterSet,
    right: InlineAsmRegisterSet,
) -> InlineAsmRegisterSet {
    InlineAsmRegisterSet {
        a: left.a && !right.a,
        x: left.x && !right.x,
        y: left.y && !right.y,
        flags: left.flags && !right.flags,
        sp: left.sp && !right.sp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(source: &str) -> Vec<u8> {
        let program = assemble(source, 0, InlineAsmMode::Analyzed).unwrap();
        program
            .items
            .into_iter()
            .map(|item| match item {
                MachineItem::Number(number) => number.value.unwrap() as u8,
                other => panic!("unexpected relocation in numeric test: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn assembles_numeric_mads_subset() {
        assert_eq!(
            bytes("lda #$12\nsta $D01A\nlda $80\nsta.z $81\nrts\n"),
            vec![0xA9, 0x12, 0x8D, 0x1A, 0xD0, 0xA5, 0x80, 0x85, 0x81, 0x60]
        );
    }

    #[test]
    fn resolves_local_labels_and_anonymous_branches() {
        assert_eq!(
            bytes("loop:\n  dex\n  bne loop\n@:\n  beq @+\n  nop\n@:\n  rts\n"),
            vec![0xCA, 0xD0, 0xFD, 0xF0, 0x01, 0xEA, 0x60]
        );
    }

    #[test]
    fn binds_mads_self_modification_labels_to_operand_bytes() {
        let program = assemble(
            "lda immediate:#$00\nlda source:$ff00,y\nsta immediate\nsta source+1\n",
            0,
            InlineAsmMode::Analyzed,
        )
        .unwrap();

        assert_eq!(
            program.bytes,
            vec![
                0xA9, 0x00, 0xB9, 0x00, 0xFF, 0x8D, 0x00, 0x00, 0x8D, 0x00, 0x00
            ]
        );
        assert!(matches!(
            program.relocations.as_slice(),
            [
                InlineAsmRelocation {
                    target: InlineAsmRelocationTarget::Absolute(0xff00),
                    ..
                },
                InlineAsmRelocation {
                    target: InlineAsmRelocationTarget::InlineOffset(1),
                    symbol_use: InlineAsmSymbolUse::Write,
                    ..
                },
                InlineAsmRelocation {
                    target: InlineAsmRelocationTarget::InlineOffset(4),
                    symbol_use: InlineAsmSymbolUse::Write,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn rejects_self_modification_labels_without_operand_bytes() {
        let diagnostics = assemble("nop patch:\n", 0, InlineAsmMode::Analyzed).unwrap_err();
        assert!(diagnostics[0].message.contains("encoded operand"));

        let diagnostics = assemble("asl patch:a\n", 0, InlineAsmMode::Analyzed).unwrap_err();
        assert!(diagnostics[0].message.contains("encoded operand"));
    }

    #[test]
    fn rejects_external_relative_branch() {
        let diagnostics = assemble("bne ActionLabel", 10, InlineAsmMode::Analyzed).unwrap_err();
        assert!(diagnostics[0].message.contains("same ASM block"));
    }

    #[test]
    fn emits_external_low_high_and_word_relocations() {
        let program = assemble(
            "lda #<pixels\nsta ptr\nlda #>pixels\nsta ptr+1\njsr Draw\n",
            0,
            InlineAsmMode::Analyzed,
        )
        .unwrap();
        assert!(matches!(
            program.items[1],
            MachineItem::AddressExpr(MachineAddressExpr {
                selector: Some(AddressByteSelector::Low),
                ..
            })
        ));
        assert!(matches!(
            program.items[3],
            MachineItem::AddressExpr(MachineAddressExpr { selector: None, .. })
        ));
    }

    #[test]
    fn machine_state_distinguishes_incoming_and_defined_accumulator() {
        let incoming = assemble("sta $80\n", 0, InlineAsmMode::Analyzed).unwrap();
        let defined = assemble("lda #0\nsta $80\n", 0, InlineAsmMode::Analyzed).unwrap();
        assert!(
            analyze_machine_state(
                &incoming.bytes,
                &inline_control_targets(&incoming.relocations)
            )
            .reads
            .a
        );
        assert!(
            !analyze_machine_state(
                &defined.bytes,
                &inline_control_targets(&defined.relocations)
            )
            .reads
            .a
        );
        assert!(
            analyze_machine_state(
                &defined.bytes,
                &inline_control_targets(&defined.relocations)
            )
            .clobbers
            .a
        );
    }

    #[test]
    fn numeric_memory_operands_have_absolute_effect_targets() {
        let program = assemble("lda $D20A\nsta $D01A\n", 0, InlineAsmMode::Analyzed).unwrap();
        assert_eq!(program.relocations.len(), 2);
        assert!(matches!(
            program.relocations[0].target,
            InlineAsmRelocationTarget::Absolute(0xD20A)
        ));
        assert_eq!(program.relocations[0].symbol_use, InlineAsmSymbolUse::Read);
        assert_eq!(program.relocations[1].symbol_use, InlineAsmSymbolUse::Write);
    }

    #[test]
    fn supports_optional_labels_comments_and_numeric_expression_precedence() {
        assert_eq!(
            bytes(
                "count = (1 + 2) * 3\n\
                 loop dex // optional-colon label\n\
                 bne loop\n\
                 lda #count | %10000\n\
                 /* a block\ncomment */ sta $80\n"
            ),
            vec![0xCA, 0xD0, 0xFD, 0xA9, 0x19, 0x85, 0x80]
        );
        assert_eq!(bytes("lda #'\\{RETURN}'\n"), vec![0xA9, 0x9B]);
        assert_eq!(bytes("lda #'\\{SCREEN:A}'\n"), vec![0xA9, 0x21]);
    }

    #[test]
    fn analyzed_blocks_reject_unsafe_stack_and_control_effects() {
        let stack = assemble("pha\n", 0, InlineAsmMode::Analyzed).unwrap_err();
        assert!(stack[0].message.contains("balanced stack depth"));
        assert!(assemble("pha\n", 0, InlineAsmMode::Opaque).is_ok());

        let txs = assemble("txs\n", 0, InlineAsmMode::Analyzed).unwrap_err();
        assert!(txs[0].message.contains("ASM OPAQUE"));
    }

    #[test]
    fn diagnoses_unsupported_macro_syntax_as_an_instruction() {
        let diagnostics = assemble("mva $80,$81\n", 0, InlineAsmMode::Analyzed).unwrap_err();
        assert!(diagnostics[0].message.contains("unsupported"));
    }

    #[test]
    fn colon_name_bypasses_an_assembler_local_label() {
        let program =
            assemble("value:\n  nop\n  lda :value\n", 0, InlineAsmMode::Analyzed).unwrap();
        assert!(matches!(
            program.relocations.as_slice(),
            [InlineAsmRelocation {
                target: InlineAsmRelocationTarget::Symbol(name),
                ..
            }] if name == "value"
        ));
    }

    #[test]
    fn machine_state_follows_relocated_local_jump_edges() {
        let program = assemble(
            "jmp target\n  lda #0\ntarget:\n  sta $80\n",
            0,
            InlineAsmMode::Analyzed,
        )
        .unwrap();
        let state = analyze_machine_state(
            &program.bytes,
            &inline_control_targets(&program.relocations),
        );
        assert!(state.reads.a);
    }
}
