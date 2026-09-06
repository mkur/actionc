use super::*;

impl Parser<'_> {
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
                let value = expression.parse_expr(0);
                let consumed = expression.pos;
                self.pos += consumed;
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
