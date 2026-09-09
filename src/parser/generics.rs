//! Angle brackets belong to type contexts, not to the expression lexer.
use super::*;

pub(super) fn application_end(tokens: &[Token], start: usize) -> Option<usize> {
    if !matches!(tokens.get(start)?.kind, TokenKind::Lt) {
        return None;
    }
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        match token.kind {
            TokenKind::Lt => {
                depth += 1;
                if depth > 64 {
                    return None;
                }
            }
            TokenKind::Gt => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            TokenKind::Ident(_)
            | TokenKind::Dot
            | TokenKind::Comma
            | TokenKind::LParen
            | TokenKind::RParen
            | TokenKind::Keyword(
                Keyword::Byte
                | Keyword::Char
                | Keyword::Card
                | Keyword::Int
                | Keyword::Pointer
                | Keyword::Proc
                | Keyword::Func
                | Keyword::Array,
            ) => {}
            _ => return None,
        }
    }
    None
}

pub(super) fn generic_head_end(tokens: &[Token], start: usize) -> Option<usize> {
    if !matches!(tokens.get(start)?.kind, TokenKind::Ident(_)) {
        return None;
    }
    let mut end = start + 1;
    while matches!(tokens.get(end).map(|t| &t.kind), Some(TokenKind::Dot))
        && matches!(
            tokens.get(end + 1).map(|t| &t.kind),
            Some(TokenKind::Ident(_))
        )
    {
        end += 2;
    }
    let end = application_end(tokens, end)?;
    match tokens.get(end).map(|t| &t.kind) {
        Some(TokenKind::Dot | TokenKind::RParen | TokenKind::Keyword(Keyword::Pointer)) | None => {
            Some(end)
        }
        _ => None,
    }
}

impl Parser<'_> {
    pub(super) fn collect_generic_head(&mut self, tokens: &mut Vec<Token>) -> bool {
        let Some(end) = generic_head_end(&self.tokens, self.pos) else {
            return false;
        };
        while self.pos < end {
            tokens.push(self.bump());
        }
        true
    }
    fn type_close(&mut self) {
        if self.check(TokenKind::Ge) {
            // Split only the type-context view. The caller still consumes '='.
            let token = self.peek().clone();
            self.tokens.to_mut()[self.pos] = Token {
                kind: TokenKind::Gt,
                span: Span::new(token.span.start, token.span.start + 1),
                line: token.line,
            };
            self.tokens.to_mut().insert(
                self.pos + 1,
                Token {
                    kind: TokenKind::Assign,
                    span: Span::new(token.span.start + 1, token.span.end),
                    line: token.line,
                },
            );
        }
        self.expect(TokenKind::Gt);
    }

    pub(super) fn parse_type_parameters(&mut self) -> Vec<String> {
        if !self.eat(TokenKind::Lt) {
            return Vec::new();
        }
        let mut names = Vec::new();
        loop {
            if let Some(name) = self.expect_ident() {
                names.push(name);
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.type_close();
        names
    }

    pub(super) fn parse_type_application(&mut self, definition: QualifiedName) -> TypeBase {
        if !self.eat(TokenKind::Lt) {
            return TypeBase::Named(definition);
        }
        self.type_application_depth += 1;
        if self.type_application_depth > 64 {
            self.diagnostics.push(Diagnostic::new(
                self.peek().span,
                "generic type syntax nesting exceeds 64",
            ));
            let mut depth = 1usize;
            while !self.at_eof() && depth != 0 {
                match self.bump().kind {
                    TokenKind::Lt => depth += 1,
                    TokenKind::Gt | TokenKind::Ge => depth -= 1,
                    _ => {}
                }
            }
            self.type_application_depth -= 1;
            return TypeBase::Applied {
                definition,
                arguments: Vec::new(),
            };
        }
        let mut arguments = Vec::new();
        // Syntax nesting is independently bounded before semantic instantiation.
        loop {
            if let Some(mut argument) = self.parse_type_ref() {
                if matches!(argument.base, TypeBase::Callable(_)) && self.check(TokenKind::LParen) {
                    self.parse_callable_prototype(&mut argument);
                }
                arguments.push(argument);
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.type_close();
        self.type_application_depth -= 1;
        TypeBase::Applied {
            definition,
            arguments,
        }
    }
}
