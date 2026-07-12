use crate::diagnostic::Diagnostic;
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    And,
    Array,
    Byte,
    Card,
    Char,
    Define,
    Do,
    Else,
    ElseIf,
    Exit,
    Fi,
    For,
    Func,
    If,
    Include,
    Int,
    Lsh,
    Mod,
    Module,
    Od,
    Or,
    Pointer,
    Proc,
    Record,
    Return,
    Rsh,
    Set,
    Step,
    Then,
    To,
    Type,
    Until,
    While,
    Xor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberLiteral {
    pub text: String,
    pub kind: NumberKind,
    pub value: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKind {
    Byte,
    Int,
    Card,
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Number(NumberLiteral),
    String(String),
    Char(char),
    ActioncAnnotation(String),
    Keyword(Keyword),
    Assign,
    CompoundAssign(String),
    Plus,
    Minus,
    Star,
    Slash,
    Lt,
    Gt,
    Le,
    Ge,
    Ne,
    At,
    Caret,
    Dot,
    Colon,
    Comma,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Eof,
}

impl Keyword {
    pub fn action_token_id(self) -> u8 {
        match self {
            Keyword::And => 6,
            Keyword::Array => 64,
            Keyword::Byte => 33,
            Keyword::Card => 35,
            Keyword::Char => 32,
            Keyword::Define => 38,
            Keyword::Do => 98,
            Keyword::Else => 97,
            Keyword::ElseIf => 106,
            Keyword::Exit => 83,
            Keyword::Fi => 99,
            Keyword::For => 84,
            Keyword::Func => 65,
            Keyword::If => 80,
            Keyword::Include => 67,
            Keyword::Int => 34,
            Keyword::Lsh => 15,
            Keyword::Mod => 13,
            Keyword::Module => 87,
            Keyword::Od => 100,
            Keyword::Or => 5,
            Keyword::Pointer => 69,
            Keyword::Proc => 66,
            Keyword::Record => 39,
            Keyword::Return => 82,
            Keyword::Rsh => 16,
            Keyword::Set => 68,
            Keyword::Step => 102,
            Keyword::Then => 96,
            Keyword::To => 101,
            Keyword::Type => 70,
            Keyword::Until => 88,
            Keyword::While => 81,
            Keyword::Xor => 14,
        }
    }
}

impl TokenKind {
    pub fn action_token_id(&self) -> Option<u8> {
        match self {
            TokenKind::Ident(_) => None,
            TokenKind::Number(number) => Some(number.kind.action_token_id()),
            TokenKind::String(_) => Some(128 + 5),
            TokenKind::Char(_) => Some(128 + 1),
            TokenKind::ActioncAnnotation(_) => None,
            TokenKind::Keyword(keyword) => Some(keyword.action_token_id()),
            TokenKind::Assign => Some(7),
            TokenKind::CompoundAssign(_) => None,
            TokenKind::Plus => Some(1),
            TokenKind::Minus => Some(2),
            TokenKind::Star => Some(3),
            TokenKind::Slash => Some(4),
            TokenKind::Lt => Some(11),
            TokenKind::Gt => Some(9),
            TokenKind::Le => Some(12),
            TokenKind::Ge => Some(10),
            TokenKind::Ne => Some(8),
            TokenKind::At => Some(18),
            TokenKind::Caret => Some(94),
            TokenKind::Dot => Some(23),
            TokenKind::Colon => None,
            TokenKind::Comma => Some(26),
            TokenKind::LParen => Some(25),
            TokenKind::RParen => Some(24),
            TokenKind::LBracket => Some(91),
            TokenKind::RBracket => Some(93),
            TokenKind::Eof => Some(127),
        }
    }
}

impl NumberKind {
    pub fn action_token_id(self) -> u8 {
        match self {
            NumberKind::Byte => 128 + 2,
            NumberKind::Int => 128 + 3,
            NumberKind::Card => 128 + 4,
            NumberKind::Real => 128 + 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, Vec<Diagnostic>> {
    let mut lexer = Lexer::new(input);
    let mut tokens = Vec::new();

    while let Some(token) = lexer.next_token() {
        tokens.push(token);
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(input.len(), input.len()),
    });

    if lexer.diagnostics.is_empty() {
        Ok(tokens)
    } else {
        Err(lexer.diagnostics)
    }
}

struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            diagnostics: Vec::new(),
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        self.skip_trivia();
        let start = self.pos;
        let ch = self.bump()?;

        let kind = match ch {
            'A'..='Z' | 'a'..='z' | '_' => return Some(self.lex_ident_or_keyword(start)),
            '0'..='9' | '$' => return Some(self.lex_number(start)),
            '"' | '“' | '”' => return Some(self.lex_string(start, ch)),
            '\'' => return Some(self.lex_char(start)),
            ';' if self.input[start..].starts_with(";@actionc") => {
                return Some(self.lex_actionc_annotation(start));
            }
            '=' => {
                if self.match_char('=') {
                    self.lex_compound_assign(start)
                } else {
                    TokenKind::Assign
                }
            }
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '&' => TokenKind::Keyword(Keyword::And),
            '%' => TokenKind::Keyword(Keyword::Or),
            '!' => TokenKind::Keyword(Keyword::Xor),
            '#' => TokenKind::Ne,
            '<' => {
                if self.match_char('=') {
                    TokenKind::Le
                } else if self.match_char('>') {
                    TokenKind::Ne
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.match_char('=') {
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            '@' => TokenKind::At,
            '^' => TokenKind::Caret,
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon,
            ',' => TokenKind::Comma,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    Span::new(start, self.pos),
                    format!("unexpected character `{ch}`"),
                ));
                return self.next_token();
            }
        };

        Some(Token {
            kind,
            span: Span::new(start, self.pos),
        })
    }

    fn lex_ident_or_keyword(&mut self, start: usize) -> Token {
        while matches!(self.peek(), Some('A'..='Z' | 'a'..='z' | '0'..='9' | '_')) {
            self.bump();
        }

        let text = &self.input[start..self.pos];
        let upper = text.to_ascii_uppercase();
        let kind = match upper.as_str() {
            "AND" => TokenKind::Keyword(Keyword::And),
            "ARRAY" => TokenKind::Keyword(Keyword::Array),
            "BYTE" => TokenKind::Keyword(Keyword::Byte),
            "CARD" => TokenKind::Keyword(Keyword::Card),
            "CHAR" => TokenKind::Keyword(Keyword::Char),
            "DEFINE" => TokenKind::Keyword(Keyword::Define),
            "DO" => TokenKind::Keyword(Keyword::Do),
            "ELSE" => TokenKind::Keyword(Keyword::Else),
            "ELSEIF" => TokenKind::Keyword(Keyword::ElseIf),
            "EXIT" => TokenKind::Keyword(Keyword::Exit),
            "FI" => TokenKind::Keyword(Keyword::Fi),
            "FOR" => TokenKind::Keyword(Keyword::For),
            "FUNC" => TokenKind::Keyword(Keyword::Func),
            "IF" => TokenKind::Keyword(Keyword::If),
            "INCLUDE" => TokenKind::Keyword(Keyword::Include),
            "INT" => TokenKind::Keyword(Keyword::Int),
            "LSH" => TokenKind::Keyword(Keyword::Lsh),
            "MOD" => TokenKind::Keyword(Keyword::Mod),
            "MODULE" => TokenKind::Keyword(Keyword::Module),
            "OD" => TokenKind::Keyword(Keyword::Od),
            "OR" => TokenKind::Keyword(Keyword::Or),
            "POINTER" => TokenKind::Keyword(Keyword::Pointer),
            "PROC" => TokenKind::Keyword(Keyword::Proc),
            "RECORD" => TokenKind::Keyword(Keyword::Record),
            "RETURN" => TokenKind::Keyword(Keyword::Return),
            "RSH" => TokenKind::Keyword(Keyword::Rsh),
            "SET" => TokenKind::Keyword(Keyword::Set),
            "STEP" => TokenKind::Keyword(Keyword::Step),
            "THEN" => TokenKind::Keyword(Keyword::Then),
            "TO" => TokenKind::Keyword(Keyword::To),
            "TYPE" => TokenKind::Keyword(Keyword::Type),
            "UNTIL" => TokenKind::Keyword(Keyword::Until),
            "WHILE" => TokenKind::Keyword(Keyword::While),
            "XOR" => TokenKind::Keyword(Keyword::Xor),
            _ => TokenKind::Ident(text.to_string()),
        };

        Token {
            kind,
            span: Span::new(start, self.pos),
        }
    }

    fn lex_actionc_annotation(&mut self, start: usize) -> Token {
        while !matches!(self.peek(), None | Some('\n' | '\r')) {
            self.bump();
        }
        let text = self.input[start..self.pos]
            .strip_prefix(";@actionc")
            .unwrap_or("")
            .trim()
            .to_string();
        Token {
            kind: TokenKind::ActioncAnnotation(text),
            span: Span::new(start, self.pos),
        }
    }

    fn lex_number(&mut self, start: usize) -> Token {
        if self.input[start..].starts_with('$') {
            let digit_start = self.pos;
            while matches!(self.peek(), Some('0'..='9' | 'A'..='F' | 'a'..='f')) {
                self.bump();
            }
            let text = self.input[start..self.pos].to_string();
            let digits = &self.input[digit_start..self.pos];
            let value = if digits.is_empty() {
                self.diagnostics.push(Diagnostic::new(
                    Span::new(start, self.pos),
                    "expected hexadecimal digits after `$`",
                ));
                None
            } else {
                match u16::from_str_radix(digits, 16) {
                    Ok(value) => Some(value),
                    Err(_) => {
                        self.diagnostics.push(Diagnostic::new(
                            Span::new(start, self.pos),
                            "hexadecimal constant is too large",
                        ));
                        None
                    }
                }
            };
            return Token {
                kind: TokenKind::Number(NumberLiteral {
                    text,
                    kind: NumberKind::Card,
                    value,
                }),
                span: Span::new(start, self.pos),
            };
        } else {
            while matches!(self.peek(), Some('0'..='9')) {
                self.bump();
            }
            if self.peek() == Some('.') {
                self.bump();
                while matches!(self.peek(), Some('0'..='9')) {
                    self.bump();
                }
            }
            if matches!(self.peek(), Some('E' | 'e')) {
                self.bump();
                if matches!(self.peek(), Some('+' | '-')) {
                    self.bump();
                }
                let exponent_start = self.pos;
                while matches!(self.peek(), Some('0'..='9')) {
                    self.bump();
                }
                if exponent_start == self.pos {
                    self.diagnostics.push(Diagnostic::new(
                        Span::new(start, self.pos),
                        "expected exponent digits in real constant",
                    ));
                }
            }
        }

        let text = self.input[start..self.pos].to_string();
        let is_real = text.contains('.') || text.contains('E') || text.contains('e');
        let (kind, value) = if is_real {
            (NumberKind::Real, None)
        } else {
            match text.parse::<u32>() {
                Ok(value) if value <= u8::MAX as u32 => (NumberKind::Byte, Some(value as u16)),
                Ok(value) if value <= u16::MAX as u32 => (NumberKind::Int, Some(value as u16)),
                Ok(_) => {
                    self.diagnostics.push(Diagnostic::new(
                        Span::new(start, self.pos),
                        "decimal constant is too large",
                    ));
                    (NumberKind::Int, None)
                }
                Err(_) => {
                    self.diagnostics.push(Diagnostic::new(
                        Span::new(start, self.pos),
                        "invalid decimal constant",
                    ));
                    (NumberKind::Int, None)
                }
            }
        };

        Token {
            kind: TokenKind::Number(NumberLiteral { text, kind, value }),
            span: Span::new(start, self.pos),
        }
    }

    fn lex_string(&mut self, start: usize, quote: char) -> Token {
        let closing = if quote == '“' { '”' } else { quote };

        let mut text = String::new();

        while let Some(ch) = self.peek() {
            if ch == closing {
                self.bump();
                if self.peek() == Some(closing) {
                    text.push(closing);
                    self.bump();
                    continue;
                }
                return Token {
                    kind: TokenKind::String(text),
                    span: Span::new(start, self.pos),
                };
            }
            if ch == '\\' && self.peek_next() == Some('{') {
                text.extend(self.lex_atascii_escape());
                continue;
            }
            text.push(ch);
            self.bump();
        }

        self.diagnostics.push(Diagnostic::new(
            Span::new(start, self.pos),
            "unterminated string",
        ));
        Token {
            kind: TokenKind::String(text),
            span: Span::new(start, self.pos),
        }
    }

    fn lex_char(&mut self, start: usize) -> Token {
        let value = if self.peek() == Some('\\') && self.peek_next() == Some('{') {
            let chars = self.lex_atascii_escape();
            if chars.len() != 1 {
                self.diagnostics.push(Diagnostic::new(
                    Span::new(start, self.pos),
                    "character ATASCII escape must produce exactly one byte",
                ));
            }
            chars.into_iter().next().unwrap_or('\0')
        } else {
            self.bump().unwrap_or('\0')
        };

        Token {
            kind: TokenKind::Char(value),
            span: Span::new(start, self.pos),
        }
    }

    fn lex_atascii_escape(&mut self) -> Vec<char> {
        let start = self.pos;
        self.bump();
        self.bump();
        let body_start = self.pos;
        while !matches!(self.peek(), None | Some('}')) {
            self.bump();
        }
        let body = &self.input[body_start..self.pos];
        if !self.match_char('}') {
            self.diagnostics.push(Diagnostic::new(
                Span::new(start, self.pos),
                "unterminated ATASCII escape",
            ));
            return vec!['?'];
        }

        match decode_atascii_escape(body) {
            Ok(chars) => chars,
            Err(message) => {
                self.diagnostics
                    .push(Diagnostic::new(Span::new(start, self.pos), message));
                vec!['?']
            }
        }
    }

    fn lex_compound_assign(&mut self, start: usize) -> TokenKind {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            self.bump();
        }

        let op_start = self.pos;
        match self.peek() {
            Some('+' | '-' | '/' | '*' | '&' | '%' | '!') => {
                self.bump();
            }
            Some('A'..='Z' | 'a'..='z') => {
                while matches!(self.peek(), Some('A'..='Z' | 'a'..='z')) {
                    self.bump();
                }
            }
            _ => {}
        }

        if op_start == self.pos {
            self.diagnostics.push(Diagnostic::new(
                Span::new(start, self.pos),
                "expected operator after compound assignment `==`",
            ));
        }

        TokenKind::CompoundAssign(self.input[op_start..self.pos].to_ascii_uppercase())
    }

    fn skip_trivia(&mut self) {
        loop {
            loop {
                if matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
                    self.bump();
                } else if self.match_layout_trivia() {
                } else {
                    break;
                }
            }

            if self.input[self.pos..].starts_with(";@actionc") {
                break;
            }

            if self.peek() == Some(';') {
                while !matches!(self.peek(), None | Some('\n' | '\r')) {
                    self.bump();
                }
            } else {
                break;
            }
        }
    }

    fn match_layout_trivia(&mut self) -> bool {
        // Some original Toolkit sources use inverse underscore as visual
        // spacing in executable text. The extractor spells that byte as
        // \{INV:_}; outside literals it behaves like ordinary trivia.
        for escape in ["\\{INV:_}", "\\{inv:_}"] {
            if self.input[self.pos..].starts_with(escape) {
                self.pos += escape.len();
                return true;
            }
        }
        if self.peek() == Some(0xdfu8 as char) {
            self.bump();
            return true;
        }
        false
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn peek_next(&self) -> Option<char> {
        let mut chars = self.input[self.pos..].chars();
        chars.next()?;
        chars.next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }
}

fn decode_atascii_escape(body: &str) -> Result<Vec<char>, String> {
    if let Some(hex) = body.strip_prefix('$') {
        return decode_hex_byte(hex).map(|byte| vec![byte as char]);
    }
    if let Some(hex) = body
        .strip_prefix("CHAR:$")
        .or_else(|| body.strip_prefix("char:$"))
    {
        return decode_hex_byte(hex).map(|byte| vec![byte as char]);
    }
    if let Some(text) = body
        .strip_prefix("INV:")
        .or_else(|| body.strip_prefix("inv:"))
    {
        if text.is_empty() {
            return Err("inverse ATASCII escape requires at least one character".to_string());
        }
        let mut chars = Vec::new();
        for ch in text.chars() {
            let value = ch as u32;
            if value > 0x7f {
                return Err(format!("cannot inverse non-ASCII character `{ch}`"));
            }
            chars.push(((value as u8) | 0x80) as char);
        }
        return Ok(chars);
    }

    named_atascii_escape(body)
        .map(|byte| vec![byte as char])
        .ok_or_else(|| format!("unknown ATASCII escape `{body}`"))
}

fn decode_hex_byte(hex: &str) -> Result<u8, String> {
    if hex.len() != 2 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "ATASCII byte escape requires two hex digits, got `{hex}`"
        ));
    }
    u8::from_str_radix(hex, 16).map_err(|_| format!("invalid ATASCII byte `${hex}`"))
}

fn named_atascii_escape(name: &str) -> Option<u8> {
    match name.to_ascii_uppercase().as_str() {
        "RETURN" | "EOL" | "CR" => Some(0x9b),
        "ESC" | "ESCAPE" => Some(0x1b),
        "CLEAR" | "CLS" => Some(0x7d),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_keywords_case_insensitively() {
        let tokens = tokenize("proc Main() RETURN").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Keyword(Keyword::Proc));
        assert_eq!(tokens[1].kind, TokenKind::Ident("Main".to_string()));
        assert_eq!(tokens[4].kind, TokenKind::Keyword(Keyword::Return));
    }

    #[test]
    fn tokenizes_action_compound_assignment() {
        let tokens = tokenize("x ==LSH 1").unwrap();
        assert_eq!(tokens[1].kind, TokenKind::CompoundAssign("LSH".to_string()));
    }

    #[test]
    fn tokenizes_spaced_action_compound_assignment() {
        let tokens = tokenize("ky == +1").unwrap();
        assert_eq!(tokens[1].kind, TokenKind::CompoundAssign("+".to_string()));
        assert_number(&tokens[2].kind, "1", NumberKind::Byte, Some(1));
    }

    #[test]
    fn tokenizes_symbol_alias_compound_assignment() {
        let tokens = tokenize("x ==! 128").unwrap();
        assert_eq!(tokens[1].kind, TokenKind::CompoundAssign("!".to_string()));
    }

    #[test]
    fn tokenizes_reserved_words_from_action_tables() {
        let tokens = tokenize("INCLUDE SET RECORD").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Keyword(Keyword::Include));
        assert_eq!(tokens[1].kind, TokenKind::Keyword(Keyword::Set));
        assert_eq!(tokens[2].kind, TokenKind::Keyword(Keyword::Record));
    }

    #[test]
    fn tokenizes_symbol_aliases_like_lexchars() {
        let tokens = tokenize("! # % &").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Keyword(Keyword::Xor));
        assert_eq!(tokens[1].kind, TokenKind::Ne);
        assert_eq!(tokens[2].kind, TokenKind::Keyword(Keyword::Or));
        assert_eq!(tokens[3].kind, TokenKind::Keyword(Keyword::And));
    }

    #[test]
    fn tokenizes_action_character_constant_without_closing_quote() {
        let tokens = tokenize("'A").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Char('A'));
    }

    #[test]
    fn tokenizes_machine_block_colon_separator() {
        let tokens = tokenize("[ TSX : STX sp ]").unwrap();
        assert_eq!(tokens[2].kind, TokenKind::Colon);
        assert_eq!(tokens[2].kind.action_token_id(), None);
    }

    #[test]
    fn tokenizes_doubled_quotes_inside_strings() {
        let tokens = tokenize("\"A\"\"B\"").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::String("A\"B".to_string()));
    }

    #[test]
    fn tokenizes_named_and_hex_atascii_escapes_in_strings() {
        let tokens = tokenize("\"A\\{RETURN}\\{$9B}\\{CHAR:$1B}\"").unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::String(format!(
                "A{}{}{}",
                0x9bu8 as char, 0x9bu8 as char, 0x1bu8 as char
            ))
        );
    }

    #[test]
    fn tokenizes_inverse_atascii_escapes_in_strings() {
        let tokens = tokenize("\"\\{INV:Ab}\"").unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::String(format!("{}{}", 0xC1u8 as char, 0xE2u8 as char))
        );
    }

    #[test]
    fn tokenizes_atascii_escapes_in_character_constants() {
        let tokens = tokenize("'\\{RETURN} '\\{$41} '\\{INV:A}").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Char(0x9bu8 as char));
        assert_eq!(tokens[1].kind, TokenKind::Char(0x41u8 as char));
        assert_eq!(tokens[2].kind, TokenKind::Char(0xC1u8 as char));
    }

    #[test]
    fn preserves_actionc_annotation_comments() {
        let tokens =
            tokenize("; regular comment\n;@actionc returns A=$A0\nPROC F() RETURN").unwrap();

        assert_eq!(
            tokens[0].kind,
            TokenKind::ActioncAnnotation("returns A=$A0".to_string())
        );
        assert_eq!(tokens[1].kind, TokenKind::Keyword(Keyword::Proc));
    }

    #[test]
    fn rejects_unknown_atascii_escapes() {
        assert!(tokenize("\"\\{NOPE}\"").is_err());
        assert!(tokenize("\"\\{$123}\"").is_err());
        assert!(tokenize("'\\{INV:AB}").is_err());
    }

    #[test]
    fn treats_extracted_inverse_underscore_as_layout_trivia() {
        let tokens = tokenize("A\\{INV:_}B A\u{00df}B IF x=0 THEN\\{INV:_}RETURN FI").unwrap();

        assert_eq!(tokens[0].kind, TokenKind::Ident("A".to_string()));
        assert_eq!(tokens[1].kind, TokenKind::Ident("B".to_string()));
        assert_eq!(tokens[2].kind, TokenKind::Ident("A".to_string()));
        assert_eq!(tokens[3].kind, TokenKind::Ident("B".to_string()));
        assert_eq!(tokens[4].kind, TokenKind::Keyword(Keyword::If));
        assert_eq!(tokens[9].kind, TokenKind::Keyword(Keyword::Return));
    }

    #[test]
    fn identifiers_must_start_with_alpha_like_getnext() {
        let tokens = tokenize("_name A_B2").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Ident("_name".to_string()));
        assert_eq!(tokens[1].kind, TokenKind::Ident("A_B2".to_string()));
    }

    #[test]
    fn tokenizes_real_number_forms() {
        let tokens = tokenize("1.25 2E3 4e-5").unwrap();
        assert_number(&tokens[0].kind, "1.25", NumberKind::Real, None);
        assert_number(&tokens[1].kind, "2E3", NumberKind::Real, None);
        assert_number(&tokens[2].kind, "4e-5", NumberKind::Real, None);
    }

    #[test]
    fn classifies_numeric_literals_like_lexdig_and_lexhex() {
        let tokens = tokenize("0 255 256 65535 $0 $FF $1234").unwrap();
        assert_number(&tokens[0].kind, "0", NumberKind::Byte, Some(0));
        assert_number(&tokens[1].kind, "255", NumberKind::Byte, Some(255));
        assert_number(&tokens[2].kind, "256", NumberKind::Int, Some(256));
        assert_number(&tokens[3].kind, "65535", NumberKind::Int, Some(65535));
        assert_number(&tokens[4].kind, "$0", NumberKind::Card, Some(0));
        assert_number(&tokens[5].kind, "$FF", NumberKind::Card, Some(255));
        assert_number(&tokens[6].kind, "$1234", NumberKind::Card, Some(0x1234));
    }

    #[test]
    fn maps_numeric_literal_kinds_to_compiler_def_token_ids() {
        let tokens = tokenize("1 256 $1234 1.0").unwrap();
        assert_eq!(tokens[0].kind.action_token_id(), Some(130));
        assert_eq!(tokens[1].kind.action_token_id(), Some(131));
        assert_eq!(tokens[2].kind.action_token_id(), Some(132));
        assert_eq!(tokens[3].kind.action_token_id(), Some(134));
    }

    #[test]
    fn diagnoses_malformed_numeric_literals() {
        assert!(tokenize("$").is_err());
        assert!(tokenize("$10000").is_err());
        assert!(tokenize("65536").is_err());
        assert!(tokenize("1E").is_err());
        assert!(tokenize("1e+").is_err());
    }

    #[test]
    fn rejects_pipe_as_non_action_character() {
        assert!(tokenize("|").is_err());
    }

    #[test]
    fn maps_keywords_to_compiler_def_token_ids() {
        let cases = [
            (Keyword::And, 6),
            (Keyword::Array, 64),
            (Keyword::Byte, 33),
            (Keyword::Card, 35),
            (Keyword::Char, 32),
            (Keyword::Define, 38),
            (Keyword::Do, 98),
            (Keyword::Else, 97),
            (Keyword::ElseIf, 106),
            (Keyword::Exit, 83),
            (Keyword::Fi, 99),
            (Keyword::For, 84),
            (Keyword::Func, 65),
            (Keyword::If, 80),
            (Keyword::Include, 67),
            (Keyword::Int, 34),
            (Keyword::Lsh, 15),
            (Keyword::Mod, 13),
            (Keyword::Module, 87),
            (Keyword::Od, 100),
            (Keyword::Or, 5),
            (Keyword::Pointer, 69),
            (Keyword::Proc, 66),
            (Keyword::Record, 39),
            (Keyword::Return, 82),
            (Keyword::Rsh, 16),
            (Keyword::Set, 68),
            (Keyword::Step, 102),
            (Keyword::Then, 96),
            (Keyword::To, 101),
            (Keyword::Type, 70),
            (Keyword::Until, 88),
            (Keyword::While, 81),
            (Keyword::Xor, 14),
        ];

        for (keyword, token_id) in cases {
            assert_eq!(keyword.action_token_id(), token_id);
        }
    }

    #[test]
    fn maps_punctuation_to_compiler_def_token_ids() {
        let cases = [
            ("+", 1),
            ("-", 2),
            ("*", 3),
            ("/", 4),
            ("=", 7),
            ("#", 8),
            (">", 9),
            (">=", 10),
            ("<", 11),
            ("<=", 12),
            ("<>", 8),
            ("@", 18),
            (".", 23),
            (")", 24),
            ("(", 25),
            (",", 26),
            ("[", 91),
            ("]", 93),
            ("^", 94),
        ];

        for (source, token_id) in cases {
            let tokens = tokenize(source).unwrap();
            assert_eq!(tokens[0].kind.action_token_id(), Some(token_id));
        }
    }

    fn assert_number(
        kind: &TokenKind,
        expected_text: &str,
        expected_kind: NumberKind,
        expected_value: Option<u16>,
    ) {
        let TokenKind::Number(number) = kind else {
            panic!("expected number token, got {kind:?}");
        };
        assert_eq!(number.text, expected_text);
        assert_eq!(number.kind, expected_kind);
        assert_eq!(number.value, expected_value);
    }
}
