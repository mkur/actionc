use super::*;

/// A pattern occupies the opening line. A guard is one complete expression,
/// possibly containing multiline CASE values; its closing THEN ends its line.
pub(super) fn parse_case_header(
    tokens: &[Token],
    start: usize,
    selection_depth: usize,
) -> (usize, Vec<CaseLabel>, Option<Expr>, Vec<Diagnostic>) {
    let line = tokens[start].line;
    let line_end = tokens[start..]
        .iter()
        .position(|token| token.line != line || matches!(token.kind, TokenKind::Eof))
        .map_or(tokens.len(), |offset| start + offset);
    let mut depth = 0usize;
    let guard_at = (start + 1..line_end).find(|&index| {
        match tokens[index].kind {
            TokenKind::LParen | TokenKind::LBracket => depth += 1,
            TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && matches!(tokens[index].kind, TokenKind::Keyword(Keyword::If))
    });
    let mut diagnostics = Vec::new();
    let mut end = line_end;
    let guard = guard_at.and_then(|at| {
        let mut parser = ExprParser::new(&tokens[at + 1..]);
        parser.selection_depth = selection_depth;
        let value = parser.parse_expr(0);
        let then = at + 1 + parser.pos;
        if value.is_none() {
            diagnostics.push(Diagnostic::new(
                tokens[at].span,
                "expected CASE guard condition after IF",
            ));
        }
        diagnostics.extend(parser.diagnostics);
        if matches!(
            tokens.get(then).map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::Then))
        ) && tokens.get(then + 1).is_none_or(|token| {
            token.line != tokens[then].line || matches!(token.kind, TokenKind::Eof)
        }) {
            end = then + 1;
        } else {
            diagnostics.push(Diagnostic::new(
                tokens[start].span,
                "WHEN header must end with THEN on its own line",
            ));
        }
        value
    });
    let has_then = matches!(
        tokens.get(end - 1).map(|token| &token.kind),
        Some(TokenKind::Keyword(Keyword::Then))
    );
    if guard_at.is_none() && !has_then {
        diagnostics.push(Diagnostic::new(
            tokens[start].span,
            "WHEN header must end with THEN on its own line",
        ));
    }
    let header = &tokens[start + 1..guard_at.unwrap_or(end - usize::from(has_then))];
    let mut parser = Parser::new(header);
    let labels = parser.parse_case_labels(header, guard_at.is_some());
    diagnostics.extend(parser.diagnostics);
    (end, labels, guard, diagnostics)
}

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
        let selector = self.build_value_from_tokens(self.tokens[self.pos + 1..end - 1].to_vec());
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
            let mut guard = None;
            let labels = if self.is_case_arm_start_at(self.pos) {
                if saw_else {
                    self.diagnostics.push(Diagnostic::new(
                        self.peek().span,
                        "WHEN cannot follow CASE ELSE",
                    ));
                }
                when_count += 1;
                let (end, mut labels, mut checked_guard, diagnostics) =
                    parse_case_header(&self.tokens, self.pos, 0);
                self.diagnostics.extend(diagnostics);
                for label in &mut labels {
                    self.number_value_scopes(&mut label.low);
                    if let Some(high) = &mut label.high {
                        self.number_value_scopes(high);
                    }
                }
                if let Some(value) = &mut checked_guard {
                    self.number_value_scopes(value);
                }
                guard = checked_guard;
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
                guard,
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

    pub(super) fn parse_case_labels(&mut self, tokens: &[Token], guarded: bool) -> Vec<CaseLabel> {
        let mut nesting = 0usize;
        for token in tokens {
            match token.kind {
                TokenKind::LParen => nesting += 1,
                TokenKind::RParen => nesting = nesting.saturating_sub(1),
                _ => {}
            }
            if nesting > 64 {
                self.diagnostics.push(Diagnostic::new(
                    token.span,
                    "CASE pattern nesting exceeds 64 levels",
                ));
                return Vec::new();
            }
        }
        let mut labels = Vec::new();
        let mut start = 0;
        let mut depth = 0usize;
        let mut generic_end = 0usize;
        for index in 0..=tokens.len() {
            if index < generic_end {
                continue;
            }
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
                        _ => {}
                    }
                }
                if matches!(part, [Token { kind: TokenKind::Ident(name), .. }] if name == "_")
                    && !guarded
                {
                    self.diagnostics.push(Diagnostic::new(
                        part[0].span,
                        "bare CASE wildcards are not supported; use ELSE or WHEN _ IF condition THEN",
                    ));
                }
                let low =
                    self.build_value_from_tokens(part[..range.unwrap_or(part.len())].to_vec());
                let high = range.map(|i| self.build_value_from_tokens(part[i + 1..].to_vec()));
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
