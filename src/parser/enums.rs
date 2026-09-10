use super::*;

impl Parser<'_> {
    pub(super) fn parse_named_const_type(&mut self) -> Option<ConstDeclaredType> {
        let TokenKind::Ident(_) = &self.peek().kind else {
            return None;
        };
        let mut end = self.pos + 1;
        while matches!(
            self.tokens.get(end).map(|token| &token.kind),
            Some(TokenKind::Dot)
        ) && matches!(
            self.tokens.get(end + 1).map(|token| &token.kind),
            Some(TokenKind::Ident(_))
        ) {
            end += 2;
        }
        if !matches!(
            self.tokens.get(end).map(|token| &token.kind),
            Some(TokenKind::Ident(_))
        ) {
            return None;
        }
        let name = self.expect_ident().unwrap();
        Some(ConstDeclaredType::Named(
            self.parse_qualified_name_tail(name, &mut Vec::new()),
        ))
    }

    pub(super) fn parse_enum_members(&mut self) -> Vec<EnumMember> {
        let mut members = Vec::new();
        while !self.at_eof() && !matches!(self.peek().kind, TokenKind::RBracket) {
            let start = self.peek().span.start;
            let Some(name) = self.expect_ident() else {
                break;
            };
            let value = if self.eat(TokenKind::Assign) {
                // The ordinary expression parser owns precedence, qualified
                // names, calls/casts and parenthesized comparisons. Its consumed
                // prefix ends before the next member, independently of newlines.
                let mut expression = ExprParser::new(&self.tokens[self.pos..]);
                let mut value = expression.parse_expr(0);
                let consumed = expression.pos;
                self.diagnostics.extend(expression.diagnostics);
                self.pos += consumed;
                if let Some(value) = &mut value { self.number_value_scopes(value); }
                if value.is_none() {
                    self.diagnostics.push(Diagnostic::new(
                        self.peek().span,
                        "expected constant expression for ENUM member",
                    ));
                }
                value
            } else {
                None
            };
            members.push(EnumMember {
                name,
                value,
                span: Span::new(start, self.previous_end()),
            });
            if self.eat(TokenKind::Comma) && matches!(self.peek().kind, TokenKind::RBracket) {
                break;
            }
        }
        if members.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                self.peek().span,
                "ENUM requires at least one member",
            ));
        }
        members
    }
}
