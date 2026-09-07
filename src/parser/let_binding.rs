use super::*;

impl Parser<'_> {
    pub(super) fn is_let_start_at(&self, pos: usize) -> bool {
        self.is_contextual_at(pos, "LET")
            && matches!(self.tokens.get(pos + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_)) | Some(TokenKind::Keyword(
                    Keyword::Byte | Keyword::Char | Keyword::Card | Keyword::Int | Keyword::Proc)))
    }

    pub(super) fn parse_let_statement(&mut self) -> Stmt {
        let start = self.bump().span.start;
        let syntax_id = LexicalBlockSyntaxId(self.next_lexical_block_syntax_id);
        self.next_lexical_block_syntax_id += 1;
        let inferred = matches!(self.peek().kind, TokenKind::Ident(_))
            && matches!(self.tokens.get(self.pos + 1).map(|token| &token.kind),
                Some(TokenKind::Assign));
        let mut declared_type = if inferred { None } else { self.parse_type_ref() };
        let name = self.expect_ident().unwrap_or_else(|| "<missing LET name>".into());
        if let Some(ty) = &mut declared_type
            && matches!(ty.base, TypeBase::Callable(_)) && self.check(TokenKind::LParen)
        {
            self.parse_callable_prototype(ty);
        }
        self.expect(TokenKind::Assign);
        let value = self.collect_statement_expr();
        if matches!(value.kind, ExprKind::Missing | ExprKind::Raw | ExprKind::InitializerList(_)) {
            self.diagnostics.push(Diagnostic::new(value.span, "LET requires one value expression"));
        }
        Stmt::Let { syntax_id, name, declared_type, value,
            span: Span::new(start, self.previous_end()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_runtime_let_and_sequential_shadowing() {
        let tokens = crate::lexer::tokenize("PROC Main()\nLET value=ReadValue()\nLET value=value+1\nLET CARD limit=42\nLET SYS.LONGCARD wide=65536\nLET BYTE POINTER p=@limit\nRETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else { panic!() };
        assert!(routine.locals.is_empty());
        assert_eq!(routine.body.len(), 6);
        let Stmt::Let { syntax_id, declared_type, .. } = &routine.body[0] else { panic!() };
        assert_eq!(*syntax_id, LexicalBlockSyntaxId(0));
        assert!(declared_type.is_none());
        let Stmt::Let { declared_type: Some(ty), .. } = &routine.body[4] else { panic!() };
        assert!(ty.pointer);
    }

    #[test]
    fn let_word_remains_an_identifier() {
        let tokens = crate::lexer::tokenize("BYTE let\nPROC Let()\nRETURN\nPROC Main()\nlet=1\nLet()\nRETURN").unwrap();
        assert!(parse(&tokens).is_ok());
    }

    #[test]
    fn rejects_malformed_let() {
        for source in ["LET n=\nRETURN", "LET CARD =1", "LET n=[1 2]", "LET n=1, m=2"] {
            let tokens = crate::lexer::tokenize(&format!("PROC Main()\n{source}\nRETURN")).unwrap();
            assert!(parse(&tokens).is_err(), "{source}");
        }
    }
}
