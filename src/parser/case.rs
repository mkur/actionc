use super::*;

impl Parser<'_> {
    fn physical_line_end(&self, pos: usize) -> usize {
        let line = self.tokens[pos].line;
        (pos..self.tokens.len())
            .find(|&index| {
                self.tokens[index].line != line || matches!(self.tokens[index].kind, TokenKind::Eof)
            })
            .unwrap_or(self.tokens.len())
    }

    pub(super) fn is_case_start_at(&self, pos: usize) -> bool {
        if !self.is_contextual_at(pos, "CASE")
            || (pos > 0 && self.tokens[pos - 1].line == self.tokens[pos].line)
        {
            return false;
        }
        let end = self.physical_line_end(pos);
        end > pos + 1 && self.is_contextual_at(end - 1, "OF")
    }

    pub(super) fn is_case_arm_start_at(&self, pos: usize) -> bool {
        if self.case_depth == 0
            || !self.is_contextual_at(pos, "WHEN")
            || (pos > 0 && self.tokens[pos - 1].line == self.tokens[pos].line)
        {
            return false;
        }
        let end = self.physical_line_end(pos);
        matches!(
            self.tokens
                .get(end.wrapping_sub(1))
                .map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::Then))
        ) || !matches!(
            self.tokens.get(pos + 1).map(|token| &token.kind),
            Some(TokenKind::LParen | TokenKind::Assign | TokenKind::CompoundAssign(_))
        )
    }

    pub(super) fn parse_case_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        let end = self.physical_line_end(self.pos);
        let selector = build_expr_from_tokens(self.tokens[self.pos + 1..end - 1].to_vec());
        if matches!(selector.kind, ExprKind::Missing | ExprKind::Raw) {
            self.diagnostics.push(Diagnostic::new(
                selector.span,
                "expected CASE selector before OF",
            ));
        }
        self.pos = end;
        self.case_depth += 1;
        let mut arms = Vec::new();
        let mut saw_else = false;
        let mut when_count = 0;
        while !self.at_eof() && !self.is_bare_contextual_at(self.pos, "ESAC") {
            let arm_start = self.peek().span.start;
            let labels = if self.is_case_arm_start_at(self.pos) {
                if saw_else {
                    self.diagnostics.push(Diagnostic::new(
                        self.peek().span,
                        "WHEN cannot follow CASE ELSE",
                    ));
                }
                when_count += 1;
                let end = self.physical_line_end(self.pos);
                let has_then =
                    matches!(self.tokens[end - 1].kind, TokenKind::Keyword(Keyword::Then));
                if !has_then {
                    self.diagnostics.push(Diagnostic::new(
                        self.peek().span,
                        "WHEN header must end with THEN on its own line",
                    ));
                }
                let tokens = self.tokens[self.pos + 1..end - usize::from(has_then)].to_vec();
                let labels = self.parse_case_labels(&tokens);
                self.pos = end;
                Some(labels)
            } else if self.check_keyword(Keyword::Else) {
                if saw_else {
                    self.diagnostics
                        .push(Diagnostic::new(self.peek().span, "duplicate CASE ELSE"));
                }
                if self.physical_line_end(self.pos) != self.pos + 1 {
                    self.diagnostics.push(Diagnostic::new(
                        self.peek().span,
                        "CASE ELSE must occupy its own line",
                    ));
                }
                self.bump();
                saw_else = true;
                None
            } else {
                self.diagnostics.push(Diagnostic::new(
                    self.peek().span,
                    "expected WHEN, ELSE, or ESAC in CASE",
                ));
                // Do not consume the next routine or an enclosing construct's end.
                if self.is_routine_boundary() || self.is_structural_statement_terminator() {
                    break;
                }
                self.bump();
                continue;
            };
            let body = self.parse_statement_list_until(&[Keyword::Else]);
            let syntax_id = LexicalBlockSyntaxId(self.next_lexical_block_syntax_id);
            self.next_lexical_block_syntax_id += 1;
            arms.push(CaseArm {
                syntax_id,
                labels,
                body,
                span: Span::new(arm_start, self.previous_end()),
            });
        }
        if self.is_bare_contextual_at(self.pos, "ESAC") {
            self.bump();
        } else {
            self.diagnostics.push(Diagnostic::new(
                self.peek().span,
                "expected ESAC to close CASE",
            ));
        }
        self.case_depth -= 1;
        if when_count == 0 {
            self.diagnostics.push(Diagnostic::new(
                Span::new(start, self.previous_end()),
                "CASE requires at least one WHEN arm",
            ));
        }
        Stmt::Case {
            selector,
            arms,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_case_labels(&mut self, tokens: &[Token]) -> Vec<CaseLabel> {
        let mut nesting = 0usize;
        for token in tokens {
            match token.kind {
                TokenKind::LParen => nesting += 1,
                TokenKind::RParen => nesting = nesting.saturating_sub(1),
                _ => {},
            }
            if nesting > 64 {
                self.diagnostics.push(Diagnostic::new(token.span, "CASE pattern nesting exceeds 64 levels"));
                return Vec::new();
            }
        }
        let mut labels = Vec::new();
        let mut start = 0;
        let mut depth = 0usize;
        let mut generic_end = 0usize;
        for index in 0..=tokens.len() {
            if index < generic_end { continue; }
            if let Some(end) = super::generics::generic_head_end(tokens, index) {
                generic_end = end;
                continue;
            }
            if index < tokens.len() {
                match tokens[index].kind {
                    TokenKind::LParen | TokenKind::LBracket => depth += 1,
                    TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            if index == tokens.len()
                || (depth == 0 && matches!(tokens[index].kind, TokenKind::Comma))
            {
                let part = &tokens[start..index];
                let mut depth = 0usize;
                let mut range = None;
                for (i, token) in part.iter().enumerate() {
                    match token.kind {
                        TokenKind::LParen | TokenKind::LBracket => depth += 1,
                        TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
                        TokenKind::Keyword(Keyword::To) if depth == 0 => range = Some(i),
                        TokenKind::Keyword(Keyword::If) if depth == 0 => self.diagnostics.push(
                            Diagnostic::new(token.span, "CASE guards are not supported yet"),
                        ),
                        _ => {}
                    }
                }
                if matches!(part, [Token { kind: TokenKind::Ident(name), .. }] if name == "_") {
                    self.diagnostics.push(Diagnostic::new(
                        part[0].span,
                        "CASE wildcards are not supported; use ELSE",
                    ));
                }
                let low = build_expr_from_tokens(part[..range.unwrap_or(part.len())].to_vec());
                let high = range.map(|i| build_expr_from_tokens(part[i + 1..].to_vec()));
                let span = Span::new(
                    low.span.start,
                    high.as_ref().map_or(low.span.end, |expr| expr.span.end),
                );
                if matches!(low.kind, ExprKind::Missing | ExprKind::Raw)
                    || high
                        .as_ref()
                        .is_some_and(|expr| matches!(expr.kind, ExprKind::Missing | ExprKind::Raw))
                {
                    self.diagnostics
                        .push(Diagnostic::new(span, "expected constant CASE label"));
                }
                labels.push(CaseLabel { low, high, span });
                start = index + 1;
            }
        }
        labels
    }
}
