use super::*;

pub(super) fn starts_selection(tokens: &[Token], pos: usize) -> bool {
    let Some(token) = tokens.get(pos) else {
        return false;
    };
    if matches!(token.kind, TokenKind::Keyword(Keyword::If)) {
        return true;
    }
    if !matches!(&token.kind, TokenKind::Ident(name) if name.eq_ignore_ascii_case("CASE")) {
        return false;
    }
    let end = tokens[pos..]
        .iter()
        .position(|t| t.line != token.line || matches!(t.kind, TokenKind::Eof))
        .map_or(tokens.len(), |n| pos + n);
    end > pos + 2
        && token_can_end_expr(&tokens[end - 2])
        && matches!(tokens.get(end - 1).map(|t| &t.kind), Some(TokenKind::Ident(name)) if name.eq_ignore_ascii_case("OF"))
}

impl Parser<'_> {
    /// Consume a complete selection as one operand before statement boundary
    /// detection. Parsing tokens (not source text) also handles CASE guard IFs.
    pub(super) fn collect_selection_operand(&mut self, output: &mut Vec<Token>) -> bool {
        if output.last().is_some_and(token_can_end_expr)
            || !starts_selection(&self.tokens, self.pos)
        {
            return false;
        }
        let mut parser = ExprParser::new(&self.tokens[self.pos..]);
        parser.parse_selection();
        let count = parser.pos.max(1);
        self.diagnostics.extend(parser.diagnostics);
        let end = (self.pos + count).min(self.tokens.len() - 1);
        output.extend_from_slice(&self.tokens[self.pos..end]);
        self.pos = end;
        true
    }

    pub(super) fn build_value_expr(&mut self, tokens: Vec<Token>, span: Span) -> Expr {
        let selection = (0..tokens.len()).any(|pos| starts_selection(&tokens, pos));
        let (mut expr, diagnostics) = build_expr_checked(tokens, span);
        self.diagnostics.extend(diagnostics);
        if selection && matches!(expr.kind, ExprKind::Raw | ExprKind::Missing) {
            self.diagnostics
                .push(Diagnostic::new(span, "malformed IF/CASE value expression"));
        }
        self.number_value_scopes(&mut expr);
        expr
    }

    pub(super) fn build_value_from_tokens(&mut self, tokens: Vec<Token>) -> Expr {
        let span = match (tokens.first(), tokens.last()) {
            (Some(a), Some(b)) => Span::new(a.span.start, b.span.end),
            _ => self
                .tokens
                .get(self.pos)
                .map_or(Span::new(0, 0), |token| token.span),
        };
        self.build_value_expr(tokens, span)
    }

    pub(super) fn number_value_scopes(&mut self, expr: &mut Expr) {
        match &mut expr.kind {
            ExprKind::Selection(selection) => {
                if let SelectionExpr::Case { arms, .. } = selection.as_mut() {
                    for arm in arms {
                        arm.header.syntax_id =
                            LexicalBlockSyntaxId(self.next_lexical_block_syntax_id);
                        self.next_lexical_block_syntax_id += 1;
                    }
                }
                for expr in selection.expressions_mut() {
                    self.number_value_scopes(expr);
                }
            }
            ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => {
                self.number_value_scopes(expr)
            }
            ExprKind::Binary { left, right, .. } => {
                self.number_value_scopes(left);
                self.number_value_scopes(right);
            }
            ExprKind::Call { callee, args } => {
                self.number_value_scopes(callee);
                for arg in args {
                    self.number_value_scopes(arg);
                }
            }
            ExprKind::Index { base, index } => {
                self.number_value_scopes(base);
                self.number_value_scopes(index);
            }
            ExprKind::Field { base, .. } => self.number_value_scopes(base),
            _ => {}
        }
    }
}

impl ExprParser<'_> {
    fn selection_error(&mut self, message: &str) -> Option<Expr> {
        let span = self
            .peek()
            .or_else(|| self.tokens.last())
            .map_or(Span::new(0, 0), |t| t.span);
        self.diagnostics.push(Diagnostic::new(span, message));
        None
    }

    fn contextual(&self, name: &str) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Ident(value)) if value.eq_ignore_ascii_case(name))
    }

    pub(super) fn parse_selection(&mut self) -> Option<Expr> {
        if self.selection_depth >= 64 {
            return self.selection_error("IF/CASE expression nesting exceeds 64 levels");
        }
        self.selection_depth += 1;
        let start = self.pos;
        let selection = if self.eat(TokenKind::Keyword(Keyword::If)) {
            self.parse_if_value()
        } else {
            self.pos += 1; // contextual CASE
            self.parse_case_value()
        };
        self.selection_depth -= 1;
        if let Some(selection) = selection {
            Some(self.spanned_expr(ExprKind::Selection(Box::new(selection)), start))
        } else {
            self.selection_error("expected a complete IF/CASE expression with one value per arm")
        }
    }

    fn parse_if_value(&mut self) -> Option<SelectionExpr> {
        let mut branches = Vec::new();
        loop {
            let condition = self.parse_expr(0)?;
            if !self.eat(TokenKind::Keyword(Keyword::Then)) {
                self.selection_error("expected THEN in IF expression");
                return None;
            }
            let value = self.parse_expr(0)?;
            branches.push((condition, value));
            if !self.eat(TokenKind::Keyword(Keyword::ElseIf)) {
                break;
            }
        }
        if !self.eat(TokenKind::Keyword(Keyword::Else)) {
            self.selection_error("IF expression requires ELSE");
            return None;
        }
        let otherwise = self.parse_expr(0)?;
        if !self.eat(TokenKind::Keyword(Keyword::Fi)) {
            self.selection_error("expected FI to close IF expression");
            return None;
        }
        Some(SelectionExpr::If {
            branches,
            otherwise,
        })
    }

    fn parse_case_value(&mut self) -> Option<SelectionExpr> {
        let selector = self.parse_expr(0)?;
        if !self.contextual("OF") {
            return None;
        }
        let opening_line = self.bump()?.line;
        if self.peek()?.line == opening_line {
            return None;
        }
        let mut arms = Vec::new();
        let mut saw_else = false;
        while !self.contextual("ESAC") {
            let start = self.pos;
            let token = self.peek()?;
            let line = token.line;
            if start > 0 && self.tokens[start - 1].line == line {
                self.selection_error("CASE arm headers must begin on their own line");
                return None;
            }
            let mut end = self.tokens[start..]
                .iter()
                .position(|t| t.line != line || matches!(t.kind, TokenKind::Eof))
                .map_or(self.tokens.len(), |n| start + n);
            let (labels, guard) = if self.contextual("WHEN") && !saw_else {
                let (header_end, labels, guard, diagnostics) =
                    super::case::parse_case_header(self.tokens, start, self.selection_depth);
                if !diagnostics.is_empty() {
                    self.diagnostics.extend(diagnostics);
                    return None;
                }
                end = header_end;
                (Some(labels), guard)
            } else if self.check(TokenKind::Keyword(Keyword::Else)) && !saw_else && !arms.is_empty()
            {
                if end != start + 1 {
                    return None;
                }
                saw_else = true;
                (None, None)
            } else {
                self.selection_error("expected WHEN, ELSE or ESAC in CASE expression");
                return None;
            };
            self.pos = end;
            let value = self.parse_expr(0)?;
            let span = Span::new(self.tokens[start].span.start, value.span.end);
            arms.push(CaseValueArm {
                header: CaseArm {
                    syntax_id: LexicalBlockSyntaxId(u32::MAX),
                    labels,
                    guard,
                    body: Vec::new(),
                    span,
                },
                value,
            });
        }
        if arms.is_empty() {
            return None;
        }
        self.pos += 1;
        Some(SelectionExpr::Case { selector, arms })
    }
}
