use super::*;

impl Parser<'_> {
    pub(super) fn data_starts_operand(&self, previous: Option<&Token>) -> bool {
        // Whitespace does not delimit statements. In `x=data y=1`, the
        // `data y=` prefix is an operand followed by an assignment, not DATA.
        self.is_contextual_at(self.pos, "DATA")
            && !previous.is_some_and(token_can_end_expr)
    }

    pub(super) fn is_data_decl_at(&self, pos: usize) -> bool {
        self.is_contextual_at(pos, "DATA")
            && matches!(
                self.tokens.get(pos + 1).map(|t| &t.kind),
                Some(TokenKind::Ident(_))
            )
            && matches!(
                self.tokens.get(pos + 2).map(|t| &t.kind),
                Some(TokenKind::Assign)
            )
    }

    pub(super) fn parse_data_decl(&mut self) -> DataDecl {
        let start = self.bump().span.start;
        let name = self.expect_ident().unwrap_or_default();
        self.expect(TokenKind::Assign);
        let items = if self.check(TokenKind::LBracket) {
            let Stmt::MachineBlock { items, .. } = self.parse_machine_block_statement() else {
                unreachable!()
            };
            items
        } else {
            self.diagnostics.push(Diagnostic::new(
                self.peek().span,
                "DATA requires a literal block [...]",
            ));
            vec![]
        };
        DataDecl {
            name,
            visibility: Visibility::Private,
            initializer: DataInitializer::Literal(items),
            span: Span::new(start, self.previous_end()),
        }
    }
}
