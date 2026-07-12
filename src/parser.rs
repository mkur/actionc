use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::lexer::{Keyword, Token, TokenKind};
use crate::source::Span;
use std::collections::{HashMap, HashSet};

pub fn parse(tokens: &[Token]) -> Result<Program, Vec<Diagnostic>> {
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();

    if parser.diagnostics.is_empty() {
        Ok(program)
    } else {
        Err(parser.diagnostics)
    }
}

pub fn parse_machine_items(tokens: &[Token]) -> Result<Vec<MachineItem>, Vec<Diagnostic>> {
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_machine_block_statement();
    if !parser.at_eof() {
        parser.diagnostics.push(Diagnostic::new(
            parser.peek().span,
            "unexpected token after machine block",
        ));
    }
    if !parser.diagnostics.is_empty() {
        return Err(parser.diagnostics);
    }
    let Stmt::MachineBlock { items, .. } = stmt else {
        unreachable!("machine block parser should return a machine block statement");
    };
    Ok(items)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    known_non_type_defines: HashSet<String>,
    known_define_values: HashMap<String, String>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            diagnostics: Vec::new(),
            known_non_type_defines: HashSet::new(),
            known_define_values: HashMap::new(),
        }
    }

    fn parse_program(&mut self) -> Program {
        let mut modules = Vec::new();

        while !self.at_eof() {
            self.eat_keyword(Keyword::Module);
            modules.push(self.parse_module());
        }

        Program { modules }
    }

    fn parse_module(&mut self) -> Module {
        let mut items = Vec::new();
        let mut pending_annotations = Vec::new();

        while !self.at_eof() && !self.check_keyword(Keyword::Module) {
            if let TokenKind::ActioncAnnotation(text) = self.peek().kind.clone() {
                let span = self.bump().span;
                if let Some(annotation) = parse_actionc_annotation(&text) {
                    pending_annotations.push(annotation);
                } else if is_source_actionc_annotation(&text) {
                    pending_annotations.clear();
                } else {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        format!("unknown actionc annotation `{text}`"),
                    ));
                }
            } else if self.check_keyword(Keyword::Define) {
                pending_annotations.clear();
                items.push(Item::Define(self.parse_define()));
            } else if self.check_keyword(Keyword::Include) {
                pending_annotations.clear();
                items.push(Item::Include(self.parse_include()));
            } else if self.check_keyword(Keyword::Set) {
                pending_annotations.clear();
                items.push(Item::Set(self.parse_set()));
            } else if self.check_keyword(Keyword::Type) {
                pending_annotations.clear();
                items.push(Item::Declaration(Decl::Type(self.parse_type_decl())));
            } else if self.check_keyword(Keyword::Record) {
                pending_annotations.clear();
                items.push(Item::Declaration(Decl::Record(self.parse_record_decl())));
            } else if self.is_var_decl_start() {
                pending_annotations.clear();
                items.push(Item::Declaration(Decl::Var(self.parse_var_decl())));
            } else if self.check_keyword(Keyword::Proc) || self.is_func_decl_start() {
                let annotations = std::mem::take(&mut pending_annotations);
                items.push(Item::Routine(self.parse_routine(annotations)));
            } else if self.is_statement_start() {
                pending_annotations.clear();
                items.push(Item::Statement(self.parse_statement()));
            } else {
                pending_annotations.clear();
                let token = self.bump().clone();
                items.push(Item::Unsupported {
                    span: token.span,
                    note: format!("top-level construct starting with {:?}", token.kind),
                });
            }
        }

        Module { items }
    }

    fn parse_define(&mut self) -> DefineDecl {
        self.expect_keyword(Keyword::Define);
        let mut entries = Vec::new();

        loop {
            let start = self.peek().span.start;
            let Some(name) = self.expect_ident() else {
                break;
            };
            self.expect(TokenKind::Assign);
            let value = match &self.peek().kind {
                TokenKind::String(value) => {
                    let value = value.clone();
                    self.bump();
                    value
                }
                _ => {
                    self.diagnostics.push(Diagnostic::new(
                        self.peek().span,
                        "expected string constant in DEFINE",
                    ));
                    self.collect_expr_until(Stop::define_entry()).text
                }
            };
            if !define_value_is_type_alias(&value) {
                self.known_non_type_defines.insert(normalize_name(&name));
            }
            self.known_define_values
                .insert(normalize_name(&name), value.clone());
            entries.push(DefineEntry {
                name,
                value,
                span: Span::new(start, self.previous_end()),
            });

            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        DefineDecl { entries }
    }

    fn parse_include(&mut self) -> IncludeDirective {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::Include);
        let path = match &self.peek().kind {
            TokenKind::String(path) => {
                let path = path.clone();
                self.bump();
                path
            }
            _ => {
                self.diagnostics
                    .push(Diagnostic::new(self.peek().span, "expected include path"));
                String::new()
            }
        };

        IncludeDirective {
            path,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_set(&mut self) -> SetDirective {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::Set);
        let address = self.collect_expr_until(Stop::set_address());
        self.expect(TokenKind::Assign);
        let value = self.collect_expr_until(Stop::top_level());

        SetDirective {
            address,
            value,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_type_decl(&mut self) -> TypeDecl {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::Type);
        let name = self
            .expect_ident()
            .unwrap_or_else(|| "<missing type name>".to_string());
        self.expect(TokenKind::Assign);
        self.expect(TokenKind::LBracket);
        let fields = self.parse_field_decls_until(TokenKind::RBracket);
        self.expect(TokenKind::RBracket);

        TypeDecl {
            name,
            fields,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_record_decl(&mut self) -> RecordDecl {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::Record);
        let name = self
            .expect_ident()
            .unwrap_or_else(|| "<missing record name>".to_string());
        let fields = if self.eat(TokenKind::Assign) {
            self.expect(TokenKind::LBracket);
            let fields = self.parse_field_decls_until(TokenKind::RBracket);
            self.expect(TokenKind::RBracket);
            fields
        } else {
            Vec::new()
        };

        RecordDecl {
            name,
            fields,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_routine(&mut self, annotations: Vec<ActioncAnnotation>) -> Routine {
        let start = self.peek().span.start;
        let kind = if self.eat_keyword(Keyword::Proc) {
            RoutineKind::Proc
        } else {
            let return_type = self.parse_fund_type().unwrap_or(FundType::Card);
            self.expect_keyword(Keyword::Func);
            RoutineKind::Func { return_type }
        };

        while self.next_token_is_routine_name_after_define_directive() {
            self.bump();
        }

        let name = self
            .expect_ident()
            .unwrap_or_else(|| "<missing routine name>".to_string());

        let system_address = if self.eat(TokenKind::Assign) {
            Some(self.collect_expr_until(Stop::routine_address()))
        } else {
            None
        };

        let params = self.parse_param_list();
        let mut locals = Vec::new();
        let mut leading_body = Vec::new();
        while !self.is_routine_boundary() {
            if self.check_keyword(Keyword::Define) {
                leading_body.push(Stmt::Define(self.parse_define()));
            } else if self.current_token_is_define_directive_invocation() {
                leading_body.push(self.parse_define_directive_invocation());
            } else if self.is_decl_start() {
                locals.push(self.parse_decl());
            } else {
                break;
            }
        }

        let mut body = leading_body;
        body.extend(self.parse_statement_list_until(&[]));

        Routine {
            kind,
            name,
            system_address,
            params,
            locals,
            body,
            annotations,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_param_list(&mut self) -> Vec<VarDecl> {
        self.expect(TokenKind::LParen);
        let mut params = Vec::new();

        while !self.at_eof() && !self.check(TokenKind::RParen) {
            if self.is_var_decl_start() {
                params.push(self.parse_var_decl_until(Stop::params()));
            } else if self.eat(TokenKind::Comma) {
                continue;
            } else {
                self.diagnostics.push(Diagnostic::new(
                    self.peek().span,
                    "expected parameter declaration",
                ));
                self.bump();
            }
        }

        self.expect(TokenKind::RParen);
        params
    }

    fn parse_decl(&mut self) -> Decl {
        if self.check_keyword(Keyword::Type) {
            Decl::Type(self.parse_type_decl())
        } else if self.check_keyword(Keyword::Record) {
            Decl::Record(self.parse_record_decl())
        } else {
            Decl::Var(self.parse_var_decl())
        }
    }

    fn parse_var_decl(&mut self) -> VarDecl {
        self.parse_var_decl_until(Stop::declaration())
    }

    fn parse_var_decl_until(&mut self, stop: Stop) -> VarDecl {
        let start = self.peek().span.start;
        let ty = self.parse_type_ref().unwrap_or_else(|| TypeRef {
            base: TypeBase::Named("<missing type>".to_string()),
            pointer: false,
        });
        let storage = if self.eat_keyword(Keyword::Array) {
            VarStorage::Array
        } else {
            VarStorage::Plain
        };
        let entries = self.parse_decl_entries(&ty, storage, stop);

        VarDecl {
            ty,
            storage,
            entries,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_decl_entries(
        &mut self,
        ty: &TypeRef,
        _storage: VarStorage,
        stop: Stop,
    ) -> Vec<DeclEntry> {
        let mut entries = Vec::new();

        while !self.at_eof() && !self.check_decl_entry_stop(stop) {
            if !entries.is_empty()
                && self.check(TokenKind::Comma)
                && self.next_token_starts_decl()
                && !self.comma_continues_fund_decl(ty, stop)
                && !self.comma_continues_named_pointer_decl(ty)
                && !self.comma_continues_named_value_decl(ty)
            {
                break;
            }
            let continued = entries.is_empty() || self.eat(TokenKind::Comma);
            if !continued {
                break;
            }
            if self.check_decl_entry_stop(stop) {
                break;
            }

            let start = self.peek().span.start;
            let Some(name) = self.expect_ident() else {
                break;
            };
            let size = if self.eat(TokenKind::LParen) {
                let size = self.collect_expr_until(Stop::array_size());
                self.expect(TokenKind::RParen);
                Some(size)
            } else {
                None
            };
            let initializer = if self.eat(TokenKind::Assign) {
                Some(self.collect_initializer_expr(stop.for_initializer()))
            } else {
                None
            };
            entries.push(DeclEntry {
                name,
                size,
                initializer,
                span: Span::new(start, self.previous_end()),
            });
        }

        entries
    }

    fn comma_continues_named_pointer_decl(&self, ty: &TypeRef) -> bool {
        matches!(ty.base, TypeBase::Named(_))
            && ty.pointer
            && matches!(
                self.tokens.get(self.pos + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
    }

    fn comma_continues_named_value_decl(&self, ty: &TypeRef) -> bool {
        matches!(ty.base, TypeBase::Named(_))
            && !ty.pointer
            && matches!(
                self.tokens.get(self.pos + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
    }

    fn comma_continues_fund_decl(&self, ty: &TypeRef, stop: Stop) -> bool {
        matches!(ty.base, TypeBase::Fund(_))
            && (stop.stop_at_top_level || ty.pointer)
            && matches!(
                self.tokens.get(self.pos + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
    }

    fn parse_field_decls_until(&mut self, terminator: TokenKind) -> Vec<VarDecl> {
        let mut fields = Vec::new();
        let stop = Stop::fields(terminator.clone());

        while !self.at_eof() && !self.check(terminator.clone()) {
            if self.eat(TokenKind::Comma) {
                continue;
            }
            if self.is_var_decl_start() {
                fields.push(self.parse_var_decl_until(stop));
            } else {
                self.diagnostics.push(Diagnostic::new(
                    self.peek().span,
                    "expected field declaration",
                ));
                self.bump();
            }
        }

        fields
    }

    fn parse_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        if self.eat_keyword(Keyword::Return) {
            let expr = if self.eat(TokenKind::LParen) {
                let expr = self.collect_expr_until(Stop::return_expr());
                self.expect(TokenKind::RParen);
                Some(expr)
            } else {
                None
            };
            Stmt::Return(expr)
        } else if self.eat_keyword(Keyword::Exit) {
            Stmt::Exit {
                span: Span::new(start, self.previous_end()),
            }
        } else if self.check_keyword(Keyword::If) {
            self.parse_if_statement()
        } else if self.check_keyword(Keyword::While) {
            self.parse_while_statement()
        } else if self.check_keyword(Keyword::Do) {
            self.parse_do_statement()
        } else if self.check_keyword(Keyword::For) {
            self.parse_for_statement()
        } else if self.check_keyword(Keyword::Define) {
            Stmt::Define(self.parse_define())
        } else if self.check(TokenKind::LBracket) {
            self.parse_machine_block_statement()
        } else if self.current_token_is_define_directive_invocation() {
            self.parse_define_directive_invocation()
        } else {
            self.parse_assignment_or_call_statement(start)
        }
    }

    fn parse_define_directive_invocation(&mut self) -> Stmt {
        let token = self.bump().clone();
        Stmt::MachineBlock {
            items: Vec::new(),
            text: token_text(&token),
            span: token.span,
        }
    }

    fn parse_statement_list_until(&mut self, terminators: &[Keyword]) -> Vec<Stmt> {
        let mut body = Vec::new();

        loop {
            if self.consume_statement_separators() {
                continue;
            }
            if self.is_statement_list_terminator(terminators) {
                break;
            }

            let before = self.pos;
            body.push(self.parse_statement());
            if self.pos == before {
                self.bump();
            }
        }

        body
    }

    fn parse_if_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::If);
        let condition = self.collect_expr_until(Stop::until_keyword(&[Keyword::Then]));
        self.eat_keyword(Keyword::Then);
        let body = self.parse_statement_list_until(&[Keyword::ElseIf, Keyword::Else, Keyword::Fi]);
        let mut branches = vec![IfBranch { condition, body }];

        while self.eat_keyword(Keyword::ElseIf) {
            let condition = self.collect_expr_until(Stop::until_keyword(&[Keyword::Then]));
            self.eat_keyword(Keyword::Then);
            let body =
                self.parse_statement_list_until(&[Keyword::ElseIf, Keyword::Else, Keyword::Fi]);
            branches.push(IfBranch { condition, body });
        }

        let else_body = if self.eat_keyword(Keyword::Else) {
            self.parse_statement_list_until(&[Keyword::Fi])
        } else {
            Vec::new()
        };
        self.expect_keyword(Keyword::Fi);

        Stmt::If {
            branches,
            else_body,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_while_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::While);
        let condition = self.collect_expr_until(Stop::until_keyword(&[Keyword::Do]));
        self.eat_keyword(Keyword::Do);
        let body = self.parse_statement_list_until(&[Keyword::Od]);
        self.expect_keyword(Keyword::Od);

        Stmt::While {
            condition,
            body,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_do_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::Do);
        let body = self.parse_statement_list_until(&[Keyword::Until, Keyword::Od]);
        let condition = if self.eat_keyword(Keyword::Until) {
            Some(self.collect_expr_until(Stop::until_keyword(&[Keyword::Od])))
        } else {
            None
        };
        self.expect_keyword(Keyword::Od);

        Stmt::DoUntil {
            body,
            condition,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_for_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        self.expect_keyword(Keyword::For);
        let target = self.collect_expr_until(Stop::until_token(TokenKind::Assign));
        self.expect(TokenKind::Assign);
        let start_expr = self.collect_expr_until(Stop::until_keyword(&[Keyword::To]));
        self.expect_keyword(Keyword::To);
        let end = self.collect_expr_until(Stop::until_keyword(&[Keyword::Step, Keyword::Do]));
        let step = if self.eat_keyword(Keyword::Step) {
            Some(self.collect_expr_until(Stop::until_keyword(&[Keyword::Do])))
        } else {
            None
        };
        self.eat_keyword(Keyword::Do);
        let body = self.parse_statement_list_until(&[Keyword::Od]);
        self.expect_keyword(Keyword::Od);

        Stmt::For {
            target,
            start: start_expr,
            end,
            step,
            body,
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_machine_block_statement(&mut self) -> Stmt {
        let start = self.peek().span.start;
        self.expect(TokenKind::LBracket);
        let mut tokens = Vec::new();
        let mut items = Vec::new();

        while !self.at_eof() && !self.check(TokenKind::RBracket) {
            if self.eat(TokenKind::Colon) {
                continue;
            }
            let (item, item_tokens) = self.parse_machine_block_item();
            items.push(item);
            tokens.extend(item_tokens);
        }
        self.expect(TokenKind::RBracket);

        Stmt::MachineBlock {
            items,
            text: tokens_text(&tokens),
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_machine_block_item(&mut self) -> (MachineItem, Vec<Token>) {
        let first = self.bump().clone();
        match first.kind.clone() {
            TokenKind::Number(number) => {
                let mut tokens = vec![first];
                if let Some((offset, offset_tokens)) = self.parse_machine_block_offset() {
                    tokens.extend(offset_tokens);
                    return (
                        MachineItem::AddressExpr(MachineAddressExpr {
                            selector: None,
                            explicit_address: false,
                            atom: MachineAddressAtom::Number(number),
                            offset,
                            text: compact_tokens_text(&tokens),
                        }),
                        tokens,
                    );
                }
                (MachineItem::Number(number), tokens)
            }
            TokenKind::String(value) => (MachineItem::StringLiteral(value), vec![first]),
            TokenKind::Char(value) => (MachineItem::CharLiteral(value), vec![first]),
            TokenKind::Ident(name) => {
                let mut tokens = vec![first];
                if matches!(self.peek().kind, TokenKind::Caret) {
                    tokens.push(self.bump().clone());
                    let offset =
                        if let Some((offset, offset_tokens)) = self.parse_machine_block_offset() {
                            tokens.extend(offset_tokens);
                            offset
                        } else {
                            0
                        };
                    return (
                        MachineItem::AddressExpr(MachineAddressExpr {
                            selector: None,
                            explicit_address: false,
                            atom: MachineAddressAtom::Name(name),
                            offset,
                            text: compact_tokens_text(&tokens),
                        }),
                        tokens,
                    );
                }
                if let Some((offset, offset_tokens)) = self.parse_machine_block_offset() {
                    tokens.extend(offset_tokens);
                    return (
                        MachineItem::AddressExpr(MachineAddressExpr {
                            selector: None,
                            explicit_address: false,
                            atom: MachineAddressAtom::Name(name),
                            offset,
                            text: compact_tokens_text(&tokens),
                        }),
                        tokens,
                    );
                }
                (MachineItem::Name(name), tokens)
            }
            TokenKind::Star => {
                let mut tokens = vec![first];
                let offset =
                    if let Some((offset, offset_tokens)) = self.parse_machine_block_offset() {
                        tokens.extend(offset_tokens);
                        offset
                    } else {
                        0
                    };
                (
                    MachineItem::AddressExpr(MachineAddressExpr {
                        selector: None,
                        explicit_address: false,
                        atom: MachineAddressAtom::Current,
                        offset,
                        text: compact_tokens_text(&tokens),
                    }),
                    tokens,
                )
            }
            TokenKind::At => self.parse_machine_block_address_expr(None, true, first),
            TokenKind::Lt | TokenKind::Gt => {
                let selector = if matches!(first.kind, TokenKind::Lt) {
                    AddressByteSelector::Low
                } else {
                    AddressByteSelector::High
                };
                if matches!(self.peek().kind, TokenKind::At) {
                    let at = self.bump().clone();
                    let (mut item, mut tokens) =
                        self.parse_machine_block_address_expr(Some(selector), true, at);
                    let mut all_tokens = vec![first];
                    all_tokens.append(&mut tokens);
                    if let MachineItem::AddressExpr(expr) = &mut item {
                        expr.text = compact_tokens_text(&all_tokens);
                    }
                    return (item, all_tokens);
                }
                self.parse_machine_block_address_expr(Some(selector), false, first)
            }
            _ => {
                let raw = token_text(&first);
                (MachineItem::Raw(raw), vec![first])
            }
        }
    }

    fn parse_machine_block_address_expr(
        &mut self,
        selector: Option<AddressByteSelector>,
        explicit_address: bool,
        prefix: Token,
    ) -> (MachineItem, Vec<Token>) {
        let mut tokens = vec![prefix];
        let Some(atom) = self.parse_machine_block_atom(&mut tokens) else {
            return (MachineItem::Raw(tokens_text(&tokens)), tokens);
        };
        if matches!(atom, MachineAddressAtom::Name(_))
            && matches!(self.peek().kind, TokenKind::Caret)
        {
            tokens.push(self.bump().clone());
        }
        let offset = if let Some((offset, offset_tokens)) = self.parse_machine_block_offset() {
            tokens.extend(offset_tokens);
            offset
        } else {
            0
        };
        (
            MachineItem::AddressExpr(MachineAddressExpr {
                selector,
                explicit_address,
                atom,
                offset,
                text: compact_tokens_text(&tokens),
            }),
            tokens,
        )
    }

    fn parse_machine_block_atom(&mut self, tokens: &mut Vec<Token>) -> Option<MachineAddressAtom> {
        let token = self.peek().clone();
        match token.kind.clone() {
            TokenKind::Number(number) => {
                tokens.push(self.bump().clone());
                Some(MachineAddressAtom::Number(number))
            }
            TokenKind::Ident(name) => {
                tokens.push(self.bump().clone());
                Some(MachineAddressAtom::Name(name))
            }
            TokenKind::Star => {
                tokens.push(self.bump().clone());
                Some(MachineAddressAtom::Current)
            }
            _ => None,
        }
    }

    fn parse_machine_block_offset(&mut self) -> Option<(i32, Vec<Token>)> {
        if !matches!(self.peek().kind, TokenKind::Plus | TokenKind::Minus) {
            return None;
        }
        let next = self
            .tokens
            .get(self.pos.saturating_add(1))
            .map(|token| &token.kind)?;
        if !matches!(next, TokenKind::Number(_) | TokenKind::Ident(_)) {
            return None;
        }
        let op = self.bump().clone();
        let sign = if matches!(op.kind, TokenKind::Minus) {
            -1
        } else {
            1
        };
        let number_token = self.peek().clone();
        let value = match number_token.kind.clone() {
            TokenKind::Number(number) => sign * number.value.unwrap_or(0) as i32,
            TokenKind::Ident(_) => 0,
            _ => unreachable!("offset lookahead already checked for number or identifier"),
        };
        self.bump();
        Some((value, vec![op, number_token]))
    }

    fn parse_assignment_or_call_statement(&mut self, start: usize) -> Stmt {
        let target_tokens = self.collect_tokens_until_statement_assignment();

        if self.eat(TokenKind::Assign) {
            let value = self.collect_statement_expr();
            return Stmt::Assign {
                target: build_expr_from_tokens(target_tokens),
                value,
                span: Span::new(start, self.previous_end()),
            };
        }

        if let Some(op) = self.eat_compound_assign_op() {
            let value_start = self.peek().span.start;
            let value_tokens = self.collect_statement_expr_tokens();
            let value_end = value_tokens
                .last()
                .map(|token| token.span.end)
                .unwrap_or(value_start);
            let target = build_expr_from_tokens(target_tokens);
            let value = build_compound_assignment_expr(
                &target,
                op,
                value_tokens.clone(),
                Span::new(target.span.start, value_end),
            );
            if simple_compound_assignment_value(&value, &target, op) {
                return Stmt::CompoundAssign {
                    target,
                    op,
                    value: build_expr(value_tokens, Span::new(value_start, value_end)),
                    span: Span::new(start, self.previous_end()),
                };
            }
            return Stmt::Assign {
                target,
                value,
                span: Span::new(start, self.previous_end()),
            };
        }

        if target_tokens.is_empty() {
            let token = self.bump().clone();
            return Stmt::Unsupported {
                span: token.span,
                note: format!("statement starting with {:?}", token.kind),
            };
        }

        Stmt::Call {
            expr: build_expr_from_tokens(target_tokens),
            span: Span::new(start, self.previous_end()),
        }
    }

    fn parse_type_ref(&mut self) -> Option<TypeRef> {
        let base = if self.eat_keyword(Keyword::Proc) {
            self.expect_keyword(Keyword::Pointer);
            TypeBase::Callable(RoutineKind::Proc)
        } else if let Some(fund) = self.parse_fund_type() {
            if self.eat_keyword(Keyword::Func) {
                self.expect_keyword(Keyword::Pointer);
                TypeBase::Callable(RoutineKind::Func { return_type: fund })
            } else {
                TypeBase::Fund(fund)
            }
        } else if let Some(name) = self.expect_ident_if_present() {
            TypeBase::Named(name)
        } else {
            self.diagnostics
                .push(Diagnostic::new(self.peek().span, "expected type"));
            return None;
        };
        let pointer = !matches!(base, TypeBase::Callable(_)) && self.eat_keyword(Keyword::Pointer);

        Some(TypeRef { base, pointer })
    }

    fn parse_fund_type(&mut self) -> Option<FundType> {
        let ty = if self.eat_keyword(Keyword::Byte) {
            FundType::Byte
        } else if self.eat_keyword(Keyword::Card) {
            FundType::Card
        } else if self.eat_keyword(Keyword::Char) {
            FundType::Char
        } else if self.eat_keyword(Keyword::Int) {
            FundType::Int
        } else {
            return None;
        };

        Some(ty)
    }

    fn collect_expr_until(&mut self, stop: Stop) -> Expr {
        let start = self.peek().span.start;
        let mut end = start;
        let mut tokens = Vec::new();
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;

        while !self.at_eof() {
            if paren_depth == 0
                && bracket_depth == 0
                && ((!tokens.is_empty() && self.check_stop(stop))
                    || (tokens.is_empty() && self.check_initializer_leading_stop(stop)))
            {
                break;
            }

            let token = self.bump();
            end = token.span.end;
            match token.kind {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }

            tokens.push(token.clone());
        }

        build_expr(tokens, Span::new(start, end))
    }

    fn collect_initializer_expr(&mut self, stop: Stop) -> Expr {
        if !self.check(TokenKind::LBracket) {
            return self.collect_scalar_initializer_expr(stop);
        }

        let start = self.peek().span.start;
        let mut end = start;
        let mut tokens = Vec::new();
        let mut bracket_depth = 0usize;

        while !self.at_eof() {
            let token = self.bump();
            end = token.span.end;

            match token.kind {
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    tokens.push(token.clone());
                    if bracket_depth == 0 {
                        break;
                    }
                    continue;
                }
                _ => {}
            }

            tokens.push(token.clone());
        }

        raw_expr(tokens, Span::new(start, end))
    }

    fn collect_scalar_initializer_expr(&mut self, stop: Stop) -> Expr {
        let start = self.peek().span.start;
        let mut end = start;
        let mut tokens = Vec::new();
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;

        while !self.at_eof() {
            if paren_depth == 0
                && bracket_depth == 0
                && ((!tokens.is_empty() && self.check_stop(stop))
                    || (tokens.is_empty() && self.check_initializer_leading_stop(stop)))
            {
                break;
            }
            if paren_depth == 0
                && bracket_depth == 0
                && !tokens.is_empty()
                && self.is_statement_expr_boundary(tokens.last().unwrap())
            {
                break;
            }

            let token = self.bump();
            end = token.span.end;
            match token.kind {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }
            tokens.push(token.clone());
        }

        build_expr(tokens, Span::new(start, end))
    }

    fn check_initializer_leading_stop(&self, stop: Stop) -> bool {
        stop.tokens.iter().any(|kind| self.check(kind.clone()))
            || stop
                .keywords
                .iter()
                .any(|keyword| self.check_keyword(*keyword))
            || (stop.stop_at_top_level && self.is_routine_boundary())
    }

    fn collect_tokens_until_statement_assignment(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;

        while !self.at_eof() {
            if paren_depth == 0
                && bracket_depth == 0
                && (self.check(TokenKind::Assign)
                    || matches!(self.peek().kind, TokenKind::CompoundAssign(_)))
            {
                break;
            }
            if paren_depth == 0
                && bracket_depth == 0
                && !tokens.is_empty()
                && self.is_statement_expr_boundary(tokens.last().unwrap())
            {
                break;
            }
            if paren_depth == 0 && bracket_depth == 0 && self.is_statement_boundary() {
                break;
            }

            let token = self.bump().clone();
            match token.kind {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }
            tokens.push(token);
        }

        tokens
    }

    fn collect_statement_expr(&mut self) -> Expr {
        let start = self.peek().span.start;
        let tokens = self.collect_statement_expr_tokens();
        let end = tokens.last().map(|token| token.span.end).unwrap_or(start);

        build_expr(tokens, Span::new(start, end))
    }

    fn collect_statement_expr_tokens(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;

        while !self.at_eof() {
            if paren_depth == 0
                && bracket_depth == 0
                && !tokens.is_empty()
                && self.is_statement_expr_boundary(tokens.last().unwrap())
            {
                break;
            }
            if paren_depth == 0 && bracket_depth == 0 && self.is_statement_boundary() {
                break;
            }

            let token = self.bump().clone();
            match token.kind {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }
            tokens.push(token);
        }

        tokens
    }

    fn is_decl_start(&self) -> bool {
        self.check_keyword(Keyword::Type)
            || self.check_keyword(Keyword::Record)
            || self.is_var_decl_start()
    }

    fn next_token_starts_decl(&self) -> bool {
        self.is_decl_start_at(self.pos + 1)
    }

    fn is_var_decl_start(&self) -> bool {
        self.is_var_decl_start_at(self.pos)
    }

    fn is_decl_start_at(&self, pos: usize) -> bool {
        matches!(
            self.tokens.get(pos).map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::Type)) | Some(TokenKind::Keyword(Keyword::Record))
        ) || self.is_var_decl_start_at(pos)
    }

    fn is_var_decl_start_at(&self, pos: usize) -> bool {
        self.is_proc_pointer_decl_start_at(pos)
            || self.is_func_pointer_decl_start_at(pos)
            || (self.is_fund_type_start_at(pos)
                && !matches!(
                    self.tokens.get(pos + 1).map(|token| &token.kind),
                    Some(TokenKind::Keyword(Keyword::Func))
                ))
            || self.is_named_var_decl_start_at(pos)
    }

    fn is_proc_pointer_decl_start_at(&self, pos: usize) -> bool {
        matches!(
            (
                self.tokens.get(pos).map(|token| &token.kind),
                self.tokens.get(pos + 1).map(|token| &token.kind),
                self.tokens.get(pos + 2).map(|token| &token.kind)
            ),
            (
                Some(TokenKind::Keyword(Keyword::Proc)),
                Some(TokenKind::Keyword(Keyword::Pointer)),
                Some(TokenKind::Ident(_))
            )
        )
    }

    fn is_func_pointer_decl_start_at(&self, pos: usize) -> bool {
        self.is_fund_type_start_at(pos)
            && matches!(
                (
                    self.tokens.get(pos + 1).map(|token| &token.kind),
                    self.tokens.get(pos + 2).map(|token| &token.kind),
                    self.tokens.get(pos + 3).map(|token| &token.kind)
                ),
                (
                    Some(TokenKind::Keyword(Keyword::Func)),
                    Some(TokenKind::Keyword(Keyword::Pointer)),
                    Some(TokenKind::Ident(_))
                )
            )
    }

    fn is_named_var_decl_start_at(&self, pos: usize) -> bool {
        let Some(TokenKind::Ident(name)) = self.tokens.get(pos).map(|token| &token.kind) else {
            return false;
        };
        if self.known_non_type_defines.contains(&normalize_name(name)) {
            return false;
        }
        matches!(
            self.tokens.get(pos + 1).map(|token| &token.kind),
            Some(TokenKind::Ident(_)) | Some(TokenKind::Keyword(Keyword::Array | Keyword::Pointer))
        )
    }

    fn is_fund_type_start(&self) -> bool {
        self.is_fund_type_start_at(self.pos)
    }

    fn is_fund_type_start_at(&self, pos: usize) -> bool {
        matches!(
            self.tokens.get(pos).map(|token| &token.kind),
            Some(
                TokenKind::Keyword(Keyword::Byte)
                    | TokenKind::Keyword(Keyword::Card)
                    | TokenKind::Keyword(Keyword::Char)
                    | TokenKind::Keyword(Keyword::Int)
            )
        )
    }

    fn is_func_decl_start(&self) -> bool {
        self.is_fund_type_start()
            && matches!(
                (
                    self.tokens.get(self.pos + 1).map(|token| &token.kind),
                    self.tokens.get(self.pos + 2).map(|token| &token.kind)
                ),
                (
                    Some(TokenKind::Keyword(Keyword::Func)),
                    Some(TokenKind::Ident(_))
                )
            )
    }

    fn next_token_is_routine_name_after_define_directive(&self) -> bool {
        self.current_token_is_define_directive_invocation()
            && matches!(
                self.tokens.get(self.pos + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
    }

    fn current_token_is_define_directive_invocation(&self) -> bool {
        let Some(TokenKind::Ident(name)) = self.tokens.get(self.pos).map(|token| &token.kind)
        else {
            return false;
        };
        self.known_define_values
            .get(&normalize_name(name))
            .is_some_and(|value| define_value_is_set_directive_macro(value))
    }

    fn is_routine_boundary(&self) -> bool {
        self.check_keyword(Keyword::Module)
            || (self.check_keyword(Keyword::Proc) && !self.is_proc_pointer_decl_start_at(self.pos))
            || self.is_func_decl_start()
            || self.at_eof()
    }

    fn is_statement_body_boundary(&self) -> bool {
        self.is_routine_boundary()
            || matches!(self.peek().kind, TokenKind::ActioncAnnotation(_))
            || self.check_keyword(Keyword::Include)
            || self.check_keyword(Keyword::Set)
            || self.check_keyword(Keyword::Type)
            || self.check_keyword(Keyword::Record)
            || self.is_var_decl_start()
    }

    fn is_statement_start(&self) -> bool {
        !self.is_routine_boundary()
    }

    fn is_statement_boundary(&self) -> bool {
        self.is_routine_boundary()
            || self.check_keyword(Keyword::Include)
            || self.check_keyword(Keyword::Set)
            || self.check_keyword(Keyword::Define)
            || self.check_keyword(Keyword::Type)
            || self.check_keyword(Keyword::Record)
            || matches!(self.peek().kind, TokenKind::ActioncAnnotation(_))
            || self.is_structural_statement_terminator()
            || matches!(
                self.peek().kind,
                TokenKind::Colon | TokenKind::RParen | TokenKind::RBracket
            )
    }

    fn is_structural_statement_terminator(&self) -> bool {
        self.check_keyword(Keyword::ElseIf)
            || self.check_keyword(Keyword::Else)
            || self.check_keyword(Keyword::Fi)
            || self.check_keyword(Keyword::Od)
            || self.check_keyword(Keyword::Until)
    }

    fn is_statement_list_terminator(&self, terminators: &[Keyword]) -> bool {
        self.at_eof()
            || self.is_statement_body_boundary()
            || self.check_any_keyword(terminators)
            || self.is_structural_statement_terminator()
    }

    fn consume_statement_separators(&mut self) -> bool {
        let mut consumed = false;
        while self.eat(TokenKind::Colon) {
            consumed = true;
        }
        consumed
    }

    fn is_statement_expr_boundary(&self, previous: &Token) -> bool {
        token_can_end_expr(previous) && self.token_can_start_statement()
    }

    fn token_can_start_statement(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Ident(_)
                | TokenKind::LBracket
                | TokenKind::Keyword(Keyword::If)
                | TokenKind::Keyword(Keyword::While)
                | TokenKind::Keyword(Keyword::Do)
                | TokenKind::Keyword(Keyword::For)
                | TokenKind::Keyword(Keyword::Return)
                | TokenKind::Keyword(Keyword::Exit)
        )
    }

    fn check_any_keyword(&self, keywords: &[Keyword]) -> bool {
        keywords.iter().any(|keyword| self.check_keyword(*keyword))
    }

    fn check_stop(&self, stop: Stop) -> bool {
        if stop.stop_at_top_level && self.is_top_level_start() {
            return true;
        }
        if stop.stop_before_decl_start && self.is_decl_start() {
            return true;
        }
        if stop
            .keywords
            .iter()
            .any(|keyword| self.check_keyword(*keyword))
        {
            return true;
        }
        stop.tokens.iter().any(|kind| self.check(kind.clone()))
    }

    fn check_decl_entry_stop(&self, stop: Stop) -> bool {
        if stop.tokens.iter().any(|kind| self.check(kind.clone())) {
            return true;
        }
        if stop
            .keywords
            .iter()
            .any(|keyword| self.check_keyword(*keyword))
        {
            return true;
        }
        stop.stop_at_top_level
            && (self.is_routine_boundary()
                || matches!(self.peek().kind, TokenKind::ActioncAnnotation(_)))
    }

    fn is_top_level_start(&self) -> bool {
        matches!(self.peek().kind, TokenKind::ActioncAnnotation(_))
            || self.check_keyword(Keyword::Module)
            || self.check_keyword(Keyword::Include)
            || self.check_keyword(Keyword::Set)
            || self.check_keyword(Keyword::Define)
            || self.check_keyword(Keyword::Type)
            || self.check_keyword(Keyword::Record)
            || self.check_keyword(Keyword::Proc)
            || self.is_func_decl_start()
            || self.is_var_decl_start()
            || self.at_eof()
    }

    fn expect_ident(&mut self) -> Option<String> {
        match &self.peek().kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.bump();
                Some(name)
            }
            _ => {
                self.diagnostics
                    .push(Diagnostic::new(self.peek().span, "expected identifier"));
                None
            }
        }
    }

    fn expect_ident_if_present(&mut self) -> Option<String> {
        match &self.peek().kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.bump();
                Some(name)
            }
            _ => None,
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword) {
        if !self.eat_keyword(keyword) {
            self.diagnostics.push(Diagnostic::new(
                self.peek().span,
                format!("expected keyword {:?}", keyword),
            ));
        }
    }

    fn expect(&mut self, kind: TokenKind) {
        if !self.eat(kind.clone()) {
            self.diagnostics.push(Diagnostic::new(
                self.peek().span,
                format!("expected token {:?}", kind),
            ));
        }
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        if self.check_keyword(keyword) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn check_keyword(&self, keyword: Keyword) -> bool {
        self.check(TokenKind::Keyword(keyword))
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_compound_assign_op(&mut self) -> Option<BinaryOp> {
        let TokenKind::CompoundAssign(op) = &self.peek().kind else {
            return None;
        };
        let op = compound_assign_op(op);
        self.bump();
        op
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek().kind == kind
    }

    fn bump(&mut self) -> &'a Token {
        let token = self.peek();
        if !self.at_eof() {
            self.pos += 1;
        }
        token
    }

    fn peek(&self) -> &'a Token {
        &self.tokens[self.pos]
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn previous_end(&self) -> usize {
        self.tokens
            .get(self.pos.saturating_sub(1))
            .map(|token| token.span.end)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy)]
struct Stop {
    tokens: &'static [TokenKind],
    keywords: &'static [Keyword],
    stop_at_top_level: bool,
    stop_before_decl_start: bool,
}

impl Stop {
    fn define_entry() -> Self {
        Self {
            tokens: &[TokenKind::Comma],
            keywords: &[],
            stop_at_top_level: true,
            stop_before_decl_start: false,
        }
    }

    fn set_address() -> Self {
        Self {
            tokens: &[TokenKind::Assign],
            keywords: &[],
            stop_at_top_level: false,
            stop_before_decl_start: false,
        }
    }

    fn top_level() -> Self {
        Self {
            tokens: &[],
            keywords: &[],
            stop_at_top_level: true,
            stop_before_decl_start: false,
        }
    }

    fn routine_address() -> Self {
        Self {
            tokens: &[TokenKind::LParen],
            keywords: &[],
            stop_at_top_level: false,
            stop_before_decl_start: false,
        }
    }

    fn params() -> Self {
        Self {
            tokens: &[TokenKind::RParen],
            keywords: &[],
            stop_at_top_level: false,
            stop_before_decl_start: true,
        }
    }

    fn declaration() -> Self {
        Self {
            tokens: &[],
            keywords: &[],
            stop_at_top_level: true,
            stop_before_decl_start: true,
        }
    }

    fn fields(terminator: TokenKind) -> Self {
        match terminator {
            TokenKind::RBracket => Self {
                tokens: &[TokenKind::RBracket],
                keywords: &[],
                stop_at_top_level: false,
                stop_before_decl_start: true,
            },
            _ => Self {
                tokens: &[],
                keywords: &[],
                stop_at_top_level: false,
                stop_before_decl_start: true,
            },
        }
    }

    fn array_size() -> Self {
        Self {
            tokens: &[TokenKind::RParen],
            keywords: &[],
            stop_at_top_level: false,
            stop_before_decl_start: false,
        }
    }

    fn return_expr() -> Self {
        Self {
            tokens: &[TokenKind::RParen],
            keywords: &[],
            stop_at_top_level: false,
            stop_before_decl_start: false,
        }
    }

    fn until_token(token: TokenKind) -> Self {
        match token {
            TokenKind::Assign => Self {
                tokens: &[TokenKind::Assign],
                keywords: &[],
                stop_at_top_level: false,
                stop_before_decl_start: false,
            },
            _ => Self {
                tokens: &[],
                keywords: &[],
                stop_at_top_level: false,
                stop_before_decl_start: false,
            },
        }
    }

    fn until_keyword(keywords: &'static [Keyword]) -> Self {
        Self {
            tokens: &[],
            keywords,
            stop_at_top_level: false,
            stop_before_decl_start: false,
        }
    }

    fn for_initializer(self) -> Self {
        Self {
            tokens: &[TokenKind::Comma],
            keywords: self.keywords,
            stop_at_top_level: self.stop_at_top_level,
            stop_before_decl_start: self.stop_before_decl_start,
        }
    }
}

fn build_expr(tokens: Vec<Token>, span: Span) -> Expr {
    if tokens.is_empty() {
        return Expr {
            kind: ExprKind::Missing,
            text: String::new(),
            span,
        };
    }

    let text = tokens_text(&tokens);
    let mut parser = ExprParser::new(&tokens);
    let kind = parser
        .parse_expr(0)
        .filter(|_| parser.is_finished())
        .unwrap_or(ExprKind::Raw);

    let mut expr = Expr { kind, text, span };
    normalize_expr_spans(&mut expr, span);
    expr
}

fn define_value_is_type_alias(value: &str) -> bool {
    let Ok(tokens) = crate::lexer::tokenize(value) else {
        return false;
    };
    let kinds = tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Eof))
        .map(|token| &token.kind)
        .collect::<Vec<_>>();

    matches!(
        kinds.as_slice(),
        [TokenKind::Keyword(
            Keyword::Byte | Keyword::Card | Keyword::Char | Keyword::Int
        )] | [
            TokenKind::Keyword(Keyword::Byte | Keyword::Card | Keyword::Char | Keyword::Int),
            TokenKind::Keyword(Keyword::Array | Keyword::Pointer)
        ] | [
            TokenKind::Keyword(Keyword::Proc),
            TokenKind::Keyword(Keyword::Pointer)
        ] | [
            TokenKind::Keyword(Keyword::Byte | Keyword::Card | Keyword::Char | Keyword::Int),
            TokenKind::Keyword(Keyword::Func),
            TokenKind::Keyword(Keyword::Pointer)
        ]
    )
}

fn define_value_is_set_directive_macro(value: &str) -> bool {
    let Ok(tokens) = crate::lexer::tokenize(value) else {
        return false;
    };
    let tokens = tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Eof))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return false;
    }

    let mut index = 0usize;
    let mut saw_set = false;
    while index < tokens.len() {
        if !matches!(tokens[index].kind, TokenKind::Keyword(Keyword::Set)) {
            return false;
        }
        saw_set = true;
        index += 1;

        while index < tokens.len() && !matches!(tokens[index].kind, TokenKind::Assign) {
            if matches!(tokens[index].kind, TokenKind::Keyword(Keyword::Set)) {
                return false;
            }
            index += 1;
        }
        if index >= tokens.len() {
            return false;
        }
        index += 1;

        while index < tokens.len()
            && !matches!(tokens[index].kind, TokenKind::Keyword(Keyword::Set))
        {
            index += 1;
        }
    }
    saw_set
}

fn normalize_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

fn build_expr_from_tokens(tokens: Vec<Token>) -> Expr {
    let span = match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => Span::new(first.span.start, last.span.end),
        _ => Span::new(0, 0),
    };
    build_expr(tokens, span)
}

fn build_compound_assignment_expr(
    target: &Expr,
    op: BinaryOp,
    value_tokens: Vec<Token>,
    span: Span,
) -> Expr {
    let text = format!(
        "{}{}{}",
        target.text,
        binary_op_text(op),
        tokens_text(&value_tokens)
    );
    let mut parser = ExprParser::new(&value_tokens);
    let kind = parser
        .parse_expr_after_left(target.kind.clone(), op, binary_op_precedence(op))
        .filter(|_| parser.is_finished())
        .unwrap_or(ExprKind::Raw);

    let mut expr = Expr { kind, text, span };
    normalize_expr_spans(&mut expr, span);
    expr
}

fn simple_compound_assignment_value(expr: &Expr, target: &Expr, op: BinaryOp) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Binary {
            op: expr_op,
            left,
            ..
        } if *expr_op == op && left.kind == target.kind
    )
}

fn raw_expr(tokens: Vec<Token>, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Raw,
        text: tokens_text(&tokens),
        span,
    }
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_expr(&mut self, min_prec: u8) -> Option<ExprKind> {
        let mut left = self.parse_prefix()?;

        while let Some((op, prec)) = self.peek_binary_op() {
            if prec < min_prec {
                break;
            }
            self.pos += 1;
            let right = self.parse_expr(prec + 1)?;
            left = ExprKind::Binary {
                op,
                left: Box::new(expr_from_kind(left)),
                right: Box::new(expr_from_kind(right)),
            };
        }

        Some(left)
    }

    fn parse_expr_after_left(
        &mut self,
        left: ExprKind,
        op: BinaryOp,
        compound_prec: u8,
    ) -> Option<ExprKind> {
        let right = self.parse_expr(compound_prec + 1)?;
        let mut left = ExprKind::Binary {
            op,
            left: Box::new(expr_from_kind(left)),
            right: Box::new(expr_from_kind(right)),
        };

        while let Some((op, prec)) = self.peek_binary_op() {
            self.pos += 1;
            let right = self.parse_expr(prec + 1)?;
            left = ExprKind::Binary {
                op,
                left: Box::new(expr_from_kind(left)),
                right: Box::new(expr_from_kind(right)),
            };
        }

        Some(left)
    }

    fn parse_prefix(&mut self) -> Option<ExprKind> {
        let token = self.bump()?;
        let mut expr = match &token.kind {
            TokenKind::Number(number) => ExprKind::Number(number.clone()),
            TokenKind::String(value) => ExprKind::String(value.clone()),
            TokenKind::Char(value) => ExprKind::Char(*value),
            TokenKind::Ident(name) => ExprKind::Name(name.clone()),
            TokenKind::Keyword(keyword) if fundamental_type_from_keyword(*keyword).is_some() => {
                let fund = fundamental_type_from_keyword(*keyword)?;
                let pointer = self.eat(TokenKind::Keyword(Keyword::Pointer));
                if !self.eat(TokenKind::LParen) {
                    return None;
                }
                let inner = self.parse_expr(0)?;
                if !self.eat(TokenKind::RParen) {
                    return None;
                }
                ExprKind::Cast {
                    ty: TypeRef {
                        base: TypeBase::Fund(fund),
                        pointer,
                    },
                    expr: Box::new(expr_from_kind(inner)),
                }
            }
            TokenKind::Star => ExprKind::CurrentLocation,
            TokenKind::Plus => ExprKind::Unary {
                op: UnaryOp::Plus,
                expr: Box::new(expr_from_kind(self.parse_expr(7)?)),
            },
            TokenKind::Minus => ExprKind::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr_from_kind(self.parse_expr(7)?)),
            },
            TokenKind::At => ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: Box::new(expr_from_kind(self.parse_expr(7)?)),
            },
            TokenKind::LParen => {
                let expr = self.parse_expr(0)?;
                if !self.eat(TokenKind::RParen) {
                    return None;
                }
                expr
            }
            _ => return None,
        };

        loop {
            if self.eat(TokenKind::LParen) {
                let mut args = Vec::new();
                while !self.at_end() && !self.check(TokenKind::RParen) {
                    let arg = self.parse_expr(0)?;
                    args.push(expr_from_kind(arg));
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                if !self.eat(TokenKind::RParen) {
                    return None;
                }
                expr = ExprKind::Call {
                    callee: Box::new(expr_from_kind(expr)),
                    args,
                };
            } else if self.eat(TokenKind::LBracket) {
                let index = self.parse_expr(0)?;
                if !self.eat(TokenKind::RBracket) {
                    return None;
                }
                expr = ExprKind::Index {
                    base: Box::new(expr_from_kind(expr)),
                    index: Box::new(expr_from_kind(index)),
                };
            } else if self.eat(TokenKind::Caret) {
                expr = ExprKind::Unary {
                    op: UnaryOp::Deref,
                    expr: Box::new(expr_from_kind(expr)),
                };
            } else if self.eat(TokenKind::Dot) {
                let Some(Token {
                    kind: TokenKind::Ident(field),
                    ..
                }) = self.bump()
                else {
                    return None;
                };
                expr = ExprKind::Field {
                    base: Box::new(expr_from_kind(expr)),
                    field: field.clone(),
                };
            } else {
                break;
            }
        }

        Some(expr)
    }

    fn peek_binary_op(&self) -> Option<(BinaryOp, u8)> {
        let op = match &self.peek()?.kind {
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Sub,
            TokenKind::Star => BinaryOp::Mul,
            TokenKind::Slash => BinaryOp::Div,
            TokenKind::Assign => BinaryOp::Eq,
            TokenKind::Ne => BinaryOp::Ne,
            TokenKind::Lt => BinaryOp::Lt,
            TokenKind::Le => BinaryOp::Le,
            TokenKind::Gt => BinaryOp::Gt,
            TokenKind::Ge => BinaryOp::Ge,
            TokenKind::Keyword(Keyword::Mod) => BinaryOp::Mod,
            TokenKind::Keyword(Keyword::Lsh) => BinaryOp::Lsh,
            TokenKind::Keyword(Keyword::Rsh) => BinaryOp::Rsh,
            TokenKind::Keyword(Keyword::And) => BinaryOp::And,
            TokenKind::Keyword(Keyword::Or) => BinaryOp::Or,
            TokenKind::Keyword(Keyword::Xor) => BinaryOp::Xor,
            _ => return None,
        };
        Some((op, binary_op_precedence(op)))
    }

    fn is_finished(&self) -> bool {
        self.pos == self.tokens.len()
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Token> {
        let token = self.peek()?;
        self.pos += 1;
        Some(token)
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek().is_some_and(|token| token.kind == kind)
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

fn binary_op_precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Lsh | BinaryOp::Rsh => 6,
        BinaryOp::Add | BinaryOp::Sub => 5,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            4
        }
        BinaryOp::And => 3,
        BinaryOp::Or => 2,
        BinaryOp::Xor => 1,
    }
}

fn expr_from_kind(kind: ExprKind) -> Expr {
    Expr {
        kind,
        text: String::new(),
        span: Span::new(0, 0),
    }
}

fn normalize_expr_spans(expr: &mut Expr, fallback: Span) {
    if expr.span == Span::new(0, 0) {
        expr.span = fallback;
    }
    let span = expr.span;
    match &mut expr.kind {
        ExprKind::Unary { expr, .. } => normalize_expr_spans(expr, span),
        ExprKind::Cast { expr, .. } => normalize_expr_spans(expr, span),
        ExprKind::Binary { left, right, .. } => {
            normalize_expr_spans(left, span);
            normalize_expr_spans(right, span);
        }
        ExprKind::Call { callee, args } => {
            normalize_expr_spans(callee, span);
            for arg in args {
                normalize_expr_spans(arg, span);
            }
        }
        ExprKind::Index { base, index } => {
            normalize_expr_spans(base, span);
            normalize_expr_spans(index, span);
        }
        ExprKind::Field { base, .. } => normalize_expr_spans(base, span),
        ExprKind::Missing
        | ExprKind::Raw
        | ExprKind::CurrentLocation
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Char(_)
        | ExprKind::Name(_) => {}
    }
}

fn fundamental_type_from_keyword(keyword: Keyword) -> Option<FundType> {
    match keyword {
        Keyword::Byte => Some(FundType::Byte),
        Keyword::Card => Some(FundType::Card),
        Keyword::Char => Some(FundType::Char),
        Keyword::Int => Some(FundType::Int),
        _ => None,
    }
}

fn compound_assign_op(op: &str) -> Option<BinaryOp> {
    match op {
        "+" => Some(BinaryOp::Add),
        "-" => Some(BinaryOp::Sub),
        "*" => Some(BinaryOp::Mul),
        "/" => Some(BinaryOp::Div),
        "&" | "AND" => Some(BinaryOp::And),
        "%" | "OR" => Some(BinaryOp::Or),
        "!" | "XOR" => Some(BinaryOp::Xor),
        "MOD" | "REM" => Some(BinaryOp::Mod),
        "LSH" => Some(BinaryOp::Lsh),
        "RSH" => Some(BinaryOp::Rsh),
        _ => None,
    }
}

fn binary_op_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => " MOD ",
        BinaryOp::Lsh => " LSH ",
        BinaryOp::Rsh => " RSH ",
        BinaryOp::And => " AND ",
        BinaryOp::Or => " OR ",
        BinaryOp::Xor => " XOR ",
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "<>",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
    }
}

fn token_can_end_expr(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Ident(_)
            | TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::Char(_)
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::Caret
    )
}

fn tokens_text(tokens: &[Token]) -> String {
    let mut text = String::new();
    for token in tokens {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&token_text(token));
    }
    text
}

fn compact_tokens_text(tokens: &[Token]) -> String {
    tokens.iter().map(token_text).collect::<String>()
}

fn token_text(token: &Token) -> String {
    match &token.kind {
        TokenKind::Ident(text) => text.clone(),
        TokenKind::Number(number) => number.text.clone(),
        TokenKind::String(text) => format!("\"{text}\""),
        TokenKind::Char(ch) => format!("'{ch}"),
        TokenKind::ActioncAnnotation(text) => format!(";@actionc {text}"),
        TokenKind::Keyword(keyword) => format!("{keyword:?}").to_ascii_uppercase(),
        TokenKind::Assign => "=".to_string(),
        TokenKind::CompoundAssign(op) => format!("=={op}"),
        TokenKind::Plus => "+".to_string(),
        TokenKind::Minus => "-".to_string(),
        TokenKind::Star => "*".to_string(),
        TokenKind::Slash => "/".to_string(),
        TokenKind::Lt => "<".to_string(),
        TokenKind::Gt => ">".to_string(),
        TokenKind::Le => "<=".to_string(),
        TokenKind::Ge => ">=".to_string(),
        TokenKind::Ne => "<>".to_string(),
        TokenKind::At => "@".to_string(),
        TokenKind::Caret => "^".to_string(),
        TokenKind::Dot => ".".to_string(),
        TokenKind::Colon => ":".to_string(),
        TokenKind::Comma => ",".to_string(),
        TokenKind::LParen => "(".to_string(),
        TokenKind::RParen => ")".to_string(),
        TokenKind::LBracket => "[".to_string(),
        TokenKind::RBracket => "]".to_string(),
        TokenKind::Eof => "<eof>".to_string(),
    }
}

fn parse_actionc_annotation(text: &str) -> Option<ActioncAnnotation> {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    match normalized.as_str() {
        "RETURNS A=$A0" => Some(ActioncAnnotation::ReturnsAEqualsA0),
        "PROFILE COMPAT" | "PROFILE LEGACY" => Some(ActioncAnnotation::DebugProfileCompat),
        _ if normalized.starts_with("WRITES ") => parse_writes_annotation(&normalized),
        _ => parse_effect_annotation(&normalized),
    }
}

fn is_source_actionc_annotation(text: &str) -> bool {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    matches!(
        normalized.as_str(),
        "PROFILE MODERN" | "BACKEND CLASSIC" | "BACKEND MIR6502"
    )
}

fn parse_writes_annotation(text: &str) -> Option<ActioncAnnotation> {
    let (_, items) = text.split_once(' ')?;
    let mut addresses = AnnotationAddressRanges::default();
    for item in items.split(' ') {
        if let Some(range) = parse_annotation_address_range(item) {
            if !addresses.push(range) {
                return None;
            }
        } else {
            for symbol in item.split('/') {
                let symbol = parse_annotation_symbol(symbol)?;
                if addresses.symbols.len() >= 8 {
                    return None;
                }
                addresses.symbols.push(symbol.to_string());
            }
        }
    }
    Some(ActioncAnnotation::Writes { addresses })
}

fn parse_effect_annotation(text: &str) -> Option<ActioncAnnotation> {
    let (kind, items) = text.split_once(' ')?;
    if !matches!(kind, "PRESERVES" | "CLOBBERS") {
        return None;
    }
    let mut registers = AnnotationRegisterSet::default();
    let mut zero_page = AnnotationZeroPageRanges::default();
    for item in items.split(' ') {
        match item {
            "A" => registers.a = true,
            "X" => registers.x = true,
            "Y" => registers.y = true,
            _ => {
                if let Some(range) = parse_annotation_zero_page_range(item) {
                    if !zero_page.push(range) {
                        return None;
                    }
                } else {
                    for symbol in item.split('/') {
                        let symbol = parse_annotation_symbol(symbol)?;
                        if zero_page.symbols.len() >= 8 {
                            return None;
                        }
                        zero_page.symbols.push(symbol.to_string());
                    }
                }
            }
        }
    }
    Some(match kind {
        "PRESERVES" => ActioncAnnotation::Preserves {
            registers,
            zero_page,
        },
        "CLOBBERS" => ActioncAnnotation::Clobbers {
            registers,
            zero_page,
        },
        _ => unreachable!(),
    })
}

fn parse_annotation_zero_page_range(text: &str) -> Option<AnnotationZeroPageRange> {
    let (start, end) = if let Some((start, end)) = text.split_once('-') {
        (start, end)
    } else if let Some((start, end)) = text.split_once('/') {
        (start, end)
    } else {
        (text, text)
    };
    let start = parse_annotation_hex_byte(start)?;
    let end = parse_annotation_hex_byte(end)?;
    (start <= end).then_some(AnnotationZeroPageRange { start, end })
}

fn parse_annotation_hex_byte(text: &str) -> Option<u8> {
    let hex = text.trim().strip_prefix('$')?;
    (hex.len() == 2)
        .then(|| u8::from_str_radix(hex, 16).ok())
        .flatten()
}

fn parse_annotation_symbol(text: &str) -> Option<&str> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    chars
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        .then_some(text)
}

fn parse_annotation_address_range(text: &str) -> Option<AnnotationAddressRange> {
    let (start, end) = if let Some((start, end)) = text.split_once('-') {
        (start, end)
    } else if let Some((start, end)) = text.split_once('/') {
        (start, end)
    } else {
        (text, text)
    };
    let start = parse_annotation_hex_word(start)?;
    let end = parse_annotation_hex_word(end)?;
    (start <= end).then_some(AnnotationAddressRange { start, end })
}

fn parse_annotation_hex_word(text: &str) -> Option<u16> {
    let hex = text.trim().strip_prefix('$')?;
    (hex.len() == 4)
        .then(|| u16::from_str_radix(hex, 16).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    #[test]
    fn parses_minimal_proc() {
        let tokens = tokenize("PROC Main() RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        assert_eq!(program.modules.len(), 1);
        assert_eq!(program.modules[0].items.len(), 1);
    }

    #[test]
    fn attaches_actionc_annotations_to_next_routine() {
        let tokens = tokenize(
            ";@actionc preserves $AE-$AF\n;@actionc returns A=$A0\nBYTE FUNC F() [$85 $A0 $60]",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        assert_eq!(
            routine.annotations,
            vec![
                ActioncAnnotation::Preserves {
                    registers: AnnotationRegisterSet::default(),
                    zero_page: AnnotationZeroPageRanges {
                        ranges: [
                            Some(AnnotationZeroPageRange {
                                start: 0xAE,
                                end: 0xAF,
                            }),
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ],
                        symbols: Vec::new(),
                    },
                },
                ActioncAnnotation::ReturnsAEqualsA0,
            ]
        );
    }

    #[test]
    fn parses_actionc_effect_annotation_lists() {
        let tokens = tokenize(
            ";@actionc preserves A X Y $AC/$AD $AE-$AF\n;@actionc clobbers $A0/$A1\nPROC F() RETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let ActioncAnnotation::Preserves {
            registers,
            zero_page,
        } = &routine.annotations[0]
        else {
            panic!("expected preserves annotation");
        };
        assert!(registers.a && registers.x && registers.y);
        assert_eq!(
            zero_page.ranges[0],
            Some(AnnotationZeroPageRange {
                start: 0xAC,
                end: 0xAD,
            })
        );
        assert_eq!(
            zero_page.ranges[1],
            Some(AnnotationZeroPageRange {
                start: 0xAE,
                end: 0xAF,
            })
        );
        assert!(matches!(
            routine.annotations[1],
            ActioncAnnotation::Clobbers { .. }
        ));
    }

    #[test]
    fn parses_actionc_debug_profile_annotation() {
        let tokens = tokenize(";@actionc profile compat\nPROC F() RETURN").unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        assert_eq!(
            routine.annotations,
            vec![ActioncAnnotation::DebugProfileCompat]
        );

        let tokens = tokenize(";@actionc profile legacy\nPROC F() RETURN").unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        assert_eq!(
            routine.annotations,
            vec![ActioncAnnotation::DebugProfileCompat]
        );
    }

    #[test]
    fn accepts_source_level_actionc_codegen_annotations() {
        let tokens = tokenize(
            ";@actionc profile modern\n;@actionc backend mir6502\nBYTE x\nPROC F() RETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();

        assert_eq!(program.modules[0].items.len(), 2);
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        assert!(routine.annotations.is_empty());
    }

    #[test]
    fn parses_actionc_symbolic_effect_annotation_items() {
        let tokens = tokenize(";@actionc clobbers kx/ky\nPROC Position=*() [$60]").unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let ActioncAnnotation::Clobbers { zero_page, .. } = &routine.annotations[0] else {
            panic!("expected clobbers annotation");
        };
        assert_eq!(zero_page.symbols, ["KX", "KY"]);
    }

    #[test]
    fn parses_actionc_writes_annotation() {
        let tokens = tokenize(";@actionc writes $0340-$03BF\nPROC F() RETURN").unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        assert_eq!(
            routine.annotations,
            vec![ActioncAnnotation::Writes {
                addresses: AnnotationAddressRanges {
                    ranges: [
                        Some(AnnotationAddressRange {
                            start: 0x0340,
                            end: 0x03BF,
                        }),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                    symbols: Vec::new(),
                }
            }]
        );
    }

    #[test]
    fn parses_actionc_symbolic_writes_annotation_items() {
        let tokens = tokenize(";@actionc writes Ioerr $0340-$03BF\nPROC F() RETURN").unwrap();
        let program = parse(&tokens).unwrap();

        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let ActioncAnnotation::Writes { addresses } = &routine.annotations[0] else {
            panic!("expected writes annotation");
        };
        assert_eq!(addresses.symbols, ["IOERR"]);
        assert_eq!(
            addresses.ranges[0],
            Some(AnnotationAddressRange {
                start: 0x0340,
                end: 0x03BF,
            })
        );
    }

    #[test]
    fn attaches_actionc_annotations_after_machine_block_routine() {
        let tokens =
            tokenize("PROC P=*()\n[$60]\n;@actionc returns A=$A0\nBYTE FUNC F() [$85 $A0 $60]")
                .unwrap();
        let program = parse(&tokens).unwrap();

        assert_eq!(program.modules[0].items.len(), 2);
        let Item::Routine(previous) = &program.modules[0].items[0] else {
            panic!("expected previous routine");
        };
        assert!(previous.annotations.is_empty());
        assert_eq!(previous.body.len(), 1);

        let Item::Routine(annotated) = &program.modules[0].items[1] else {
            panic!("expected annotated routine");
        };
        assert_eq!(
            annotated.annotations,
            vec![ActioncAnnotation::ReturnsAEqualsA0]
        );
    }

    #[test]
    fn stops_declaration_before_actionc_annotation() {
        let tokens = tokenize(
            "BYTE ARRAY allocbuf($800)=$2000\n;@actionc preserves $AC/$AD\nPROC MovePage=*() [$60]",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();

        assert_eq!(program.modules[0].items.len(), 2);
        let Item::Declaration(Decl::Var(decl)) = &program.modules[0].items[0] else {
            panic!("expected declaration");
        };
        assert_eq!(decl.entries.len(), 1);
        assert_eq!(decl.entries[0].name, "allocbuf");
        let Some(initializer) = &decl.entries[0].initializer else {
            panic!("expected initializer");
        };
        let ExprKind::Number(number) = &initializer.kind else {
            panic!("expected numeric initializer, got {initializer:?}");
        };
        assert_eq!(number.value, Some(0x2000));

        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected annotated routine");
        };
        assert_eq!(
            routine.annotations,
            vec![ActioncAnnotation::Preserves {
                registers: AnnotationRegisterSet::default(),
                zero_page: AnnotationZeroPageRanges {
                    ranges: [
                        Some(AnnotationZeroPageRange {
                            start: 0xAC,
                            end: 0xAD,
                        }),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                    symbols: Vec::new(),
                },
            }]
        );
    }

    #[test]
    fn parses_define_entries_as_strings() {
        let tokens = tokenize("DEFINE SIZE = \"20\", DEVICE = \"D:\"").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Define(define) = &program.modules[0].items[0] else {
            panic!("expected define");
        };
        assert_eq!(define.entries[0].name, "SIZE");
        assert_eq!(define.entries[0].value, "20");
        assert_eq!(define.entries[1].name, "DEVICE");
        assert_eq!(define.entries[1].value, "D:");
    }

    #[test]
    fn rejects_numeric_define_values_like_original_grammar() {
        let tokens = tokenize("DEFINE RTCLOK = 20").unwrap();
        assert!(parse(&tokens).is_err());
    }

    #[test]
    fn parses_directives_and_global_declarations() {
        let tokens = tokenize(
            "INCLUDE \"SYS.ACT\" SET $491=$20 MODULE BYTE CH=$2FC, hue=[0] CARD COLOR4=$2C8",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        assert_eq!(program.modules.len(), 2);
        assert!(matches!(program.modules[0].items[0], Item::Include(_)));
        assert!(matches!(program.modules[0].items[1], Item::Set(_)));
        assert!(matches!(program.modules[1].items[0], Item::Declaration(_)));
        assert!(matches!(program.modules[1].items[1], Item::Declaration(_)));
    }

    #[test]
    fn parses_arrays_pointers_and_routine_signature() {
        let tokens = tokenize(
            "BYTE ARRAY Buf(10) CARD POINTER Ptr PROC Copy(BYTE POINTER dest, src, CARD size) RETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        assert_eq!(program.modules[0].items.len(), 3);

        let Item::Routine(routine) = &program.modules[0].items[2] else {
            panic!("expected routine");
        };
        assert_eq!(routine.name, "Copy");
        assert_eq!(routine.params.len(), 2);
        assert_eq!(routine.params[0].entries.len(), 2);
        assert!(routine.params[0].ty.pointer);
    }

    #[test]
    fn parses_named_type_parameter_group_after_comma() {
        let tokens =
            tokenize("DEFINE STRING=\"CHAR ARRAY\" PROC Output(BYTE d, STRING s) RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };

        assert_eq!(routine.params.len(), 2);
        assert_eq!(routine.params[0].entries[0].name, "d");
        assert_eq!(routine.params[1].entries[0].name, "s");
    }

    #[test]
    fn parses_sized_named_type_declaration() {
        let tokens = tokenize("DEFINE STRING=\"CHAR ARRAY\" STRING copy_right(0)=\"ACS\"").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Declaration(Decl::Var(decl)) = &program.modules[0].items[1] else {
            panic!("expected declaration");
        };

        assert_eq!(decl.entries[0].name, "copy_right");
        assert!(decl.entries[0].size.is_some());
    }

    #[test]
    fn parses_named_type_array_declaration() {
        let tokens = tokenize("TYPE Pair=[BYTE tag] Pair ARRAY recs(2) BYTE out").unwrap();
        let program = parse(&tokens).unwrap();
        assert_eq!(program.modules[0].items.len(), 3);
        let Item::Declaration(Decl::Var(decl)) = &program.modules[0].items[1] else {
            panic!("expected declaration");
        };

        assert!(matches!(decl.ty.base, TypeBase::Named(ref name) if name == "Pair"));
        assert_eq!(decl.storage, VarStorage::Array);
        assert_eq!(decl.entries[0].name, "recs");
        assert!(decl.entries[0].size.is_some());
    }

    #[test]
    fn parses_system_routine_address_and_locals() {
        let tokens = tokenize("PROC ChkErr=*(BYTE result) BYTE local RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        assert_eq!(routine.system_address.as_ref().unwrap().text, "*");
        assert_eq!(
            routine.system_address.as_ref().unwrap().kind,
            ExprKind::CurrentLocation
        );
        assert_eq!(routine.locals.len(), 1);
    }

    #[test]
    fn parses_set_define_markers_around_routine_headers() {
        let tokens = tokenize(
            "DEFINE RAM=\"SET $682=$E^ SET $B5=$C800 SET $E=$680^\", \
             ROM=\"SET $680=$E^ SET $B5=$5800 SET $E=$682^\" \
             PROC ROM Init() RAM BYTE x ROM x=1 RETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };

        assert_eq!(routine.name, "Init");
        assert_eq!(routine.locals.len(), 1);
        assert!(
            matches!(routine.body[0], Stmt::MachineBlock { ref items, .. } if items.is_empty())
        );
        assert!(
            matches!(routine.body[1], Stmt::MachineBlock { ref items, .. } if items.is_empty())
        );
        assert!(matches!(routine.body[2], Stmt::Assign { .. }));
    }

    #[test]
    fn parses_routine_defines_before_local_declarations() {
        let tokens = tokenize(
            "PROC Throw(BYTE index) DEFINE TXS=\"$9A\" BYTE sp=$A2 IF index>=25 THEN [] FI RETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        assert_eq!(program.modules[0].items.len(), 1);
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        assert_eq!(routine.locals.len(), 1);
        let Decl::Var(local) = &routine.locals[0] else {
            panic!("expected local var");
        };
        assert_eq!(local.entries[0].name, "sp");
        assert!(matches!(routine.body[0], Stmt::Define(_)));
        assert!(matches!(routine.body[1], Stmt::If { .. }));
    }

    #[test]
    fn parses_expression_precedence_into_ast() {
        let tokens = tokenize("BYTE FUNC F() RETURN(1+2*3)").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::Return(Some(expr)) = &routine.body[0] else {
            panic!("expected return expression");
        };

        let ExprKind::Binary {
            op: BinaryOp::Add,
            right,
            ..
        } = &expr.kind
        else {
            panic!("expected addition root, got {:#?}", expr.kind);
        };
        assert!(matches!(
            right.kind,
            ExprKind::Binary {
                op: BinaryOp::Mul,
                ..
            }
        ));
    }

    #[test]
    fn compound_assignment_operator_binds_above_rhs_addition() {
        let tokens = tokenize("PROC Main() x==*2+r RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::Assign { value, .. } = &routine.body[0] else {
            panic!("expected expanded assignment, got {:#?}", routine.body[0]);
        };

        let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            ..
        } = &value.kind
        else {
            panic!("expected addition root, got {:#?}", value.kind);
        };
        assert!(matches!(
            left.kind,
            ExprKind::Binary {
                op: BinaryOp::Mul,
                ..
            }
        ));
    }

    #[test]
    fn compound_assignment_addition_keeps_rhs_multiply_together() {
        let tokens = tokenize("PROC Main() x==+2*r RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::CompoundAssign {
            op: BinaryOp::Add,
            value,
            ..
        } = &routine.body[0]
        else {
            panic!(
                "expected simple compound assignment, got {:#?}",
                routine.body[0]
            );
        };

        let ExprKind::Binary {
            op: BinaryOp::Mul, ..
        } = &value.kind
        else {
            panic!("expected multiply RHS, got {:#?}", value.kind);
        };
    }

    #[test]
    fn parenthesized_compound_assignment_rhs_stays_simple_compound() {
        let tokens = tokenize("PROC Main() x==*(2+r) RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::CompoundAssign {
            op: BinaryOp::Mul,
            value,
            ..
        } = &routine.body[0]
        else {
            panic!(
                "expected simple compound assignment, got {:#?}",
                routine.body[0]
            );
        };

        assert!(matches!(
            value.kind,
            ExprKind::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn parses_calls_and_pointer_unary_into_ast() {
        let tokens = tokenize("BYTE FUNC F() RETURN(Peek(@screen))").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::Return(Some(expr)) = &routine.body[0] else {
            panic!("expected return expression");
        };

        let ExprKind::Call { callee, args } = &expr.kind else {
            panic!("expected call, got {:#?}", expr.kind);
        };
        assert!(matches!(callee.kind, ExprKind::Name(ref name) if name == "Peek"));
        assert_eq!(args.len(), 1);
        assert!(matches!(
            args[0].kind,
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                ..
            }
        ));
    }

    #[test]
    fn parses_fundamental_typed_casts_into_ast() {
        let tokens =
            tokenize("BYTE FUNC F(BYTE POINTER menu) RETURN(CHAR POINTER(menu)^)").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::Return(Some(expr)) = &routine.body[0] else {
            panic!("expected return expression");
        };

        let ExprKind::Unary {
            op: UnaryOp::Deref,
            expr: cast,
        } = &expr.kind
        else {
            panic!("expected dereference of cast, got {:#?}", expr.kind);
        };
        let ExprKind::Cast { ty, expr: inner } = &cast.kind else {
            panic!("expected cast, got {:#?}", cast.kind);
        };
        assert_eq!(ty.base, TypeBase::Fund(FundType::Char));
        assert!(ty.pointer);
        assert!(matches!(inner.kind, ExprKind::Name(ref name) if name == "menu"));
    }

    #[test]
    fn parses_function_pointer_declarations() {
        let tokens =
            tokenize("PROC POINTER handler BYTE FUNC POINTER key PROC Main() RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Declaration(Decl::Var(proc_ptr)) = &program.modules[0].items[0] else {
            panic!("expected proc pointer declaration");
        };
        let TypeBase::Callable(RoutineKind::Proc) = &proc_ptr.ty.base else {
            panic!("expected PROC POINTER type, got {:#?}", proc_ptr.ty);
        };
        let Item::Declaration(Decl::Var(func_ptr)) = &program.modules[0].items[1] else {
            panic!("expected func pointer declaration");
        };
        let TypeBase::Callable(RoutineKind::Func { return_type }) = &func_ptr.ty.base else {
            panic!("expected FUNC POINTER type, got {:#?}", func_ptr.ty);
        };
        assert_eq!(*return_type, FundType::Byte);
    }

    #[test]
    fn parses_machine_block_low_high_label_bytes() {
        let tokens = tokenize("PROC Main() [ <Target >Target ]").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::MachineBlock { items, .. } = &routine.body[0] else {
            panic!("expected machine block");
        };
        assert_eq!(
            items,
            &[
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: Some(AddressByteSelector::Low),
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("Target".to_string()),
                    offset: 0,
                    text: "<Target".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: Some(AddressByteSelector::High),
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("Target".to_string()),
                    offset: 0,
                    text: ">Target".to_string()
                })
            ]
        );
    }

    #[test]
    fn parses_machine_block_colons_as_separators() {
        let tokens = tokenize("PROC Main() [ TSX : STX sp ] RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::MachineBlock { items, .. } = &routine.body[0] else {
            panic!("expected machine block");
        };

        assert_eq!(
            items,
            &[
                MachineItem::Name("TSX".to_string()),
                MachineItem::Name("STX".to_string()),
                MachineItem::Name("sp".to_string()),
            ]
        );
    }

    #[test]
    fn parses_machine_block_caret_symbol_address() {
        let tokens =
            tokenize("PROC Main() [ screen^ <screen^ >screen^ screen^+1 screen^+OFF ]").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::MachineBlock { items, .. } = &routine.body[0] else {
            panic!("expected machine block");
        };
        assert_eq!(
            items,
            &[
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: None,
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("screen".to_string()),
                    offset: 0,
                    text: "screen^".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: Some(AddressByteSelector::Low),
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("screen".to_string()),
                    offset: 0,
                    text: "<screen^".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: Some(AddressByteSelector::High),
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("screen".to_string()),
                    offset: 0,
                    text: ">screen^".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: None,
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("screen".to_string()),
                    offset: 1,
                    text: "screen^+1".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: None,
                    explicit_address: false,
                    atom: MachineAddressAtom::Name("screen".to_string()),
                    offset: 0,
                    text: "screen^+OFF".to_string()
                })
            ]
        );
    }

    #[test]
    fn parses_machine_block_byte_stream_address_items() {
        let tokens =
            tokenize("PROC Main() [ @Target+1 <@Target >$0348 \"AB\" 'C *+5 + - ]").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::MachineBlock { items, .. } = &routine.body[0] else {
            panic!("expected machine block");
        };
        assert_eq!(
            items,
            &[
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: None,
                    explicit_address: true,
                    atom: MachineAddressAtom::Name("Target".to_string()),
                    offset: 1,
                    text: "@Target+1".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: Some(AddressByteSelector::Low),
                    explicit_address: true,
                    atom: MachineAddressAtom::Name("Target".to_string()),
                    offset: 0,
                    text: "<@Target".to_string()
                }),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: Some(AddressByteSelector::High),
                    explicit_address: false,
                    atom: MachineAddressAtom::Number(crate::lexer::NumberLiteral {
                        text: "$0348".to_string(),
                        kind: crate::lexer::NumberKind::Card,
                        value: Some(0x0348)
                    }),
                    offset: 0,
                    text: ">$0348".to_string()
                },),
                MachineItem::StringLiteral("AB".to_string()),
                MachineItem::CharLiteral('C'),
                MachineItem::AddressExpr(MachineAddressExpr {
                    selector: None,
                    explicit_address: false,
                    atom: MachineAddressAtom::Current,
                    offset: 5,
                    text: "*+5".to_string()
                }),
                MachineItem::Raw("+".to_string()),
                MachineItem::Raw("-".to_string())
            ]
        );
    }

    #[test]
    fn parsed_subexpressions_keep_diagnostic_spans() {
        let tokens = tokenize("BYTE FUNC F() RETURN(Peek(@screen)+1)").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::Return(Some(expr)) = &routine.body[0] else {
            panic!("expected return expression");
        };
        let ExprKind::Binary { left, right, .. } = &expr.kind else {
            panic!("expected binary expression");
        };
        let ExprKind::Call { callee, args } = &left.kind else {
            panic!("expected call expression");
        };

        assert_ne!(left.span, Span::new(0, 0));
        assert_ne!(callee.span, Span::new(0, 0));
        assert_ne!(args[0].span, Span::new(0, 0));
        assert_ne!(right.span, Span::new(0, 0));
    }

    #[test]
    fn parses_record_field_access_into_ast() {
        let tokens = tokenize("PROC Main() a.b=1 RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::Assign { target, .. } = &routine.body[0] else {
            panic!("expected assignment");
        };
        let ExprKind::Field { base, field } = &target.kind else {
            panic!("expected field access, got {:#?}", target.kind);
        };

        assert!(matches!(base.kind, ExprKind::Name(ref name) if name == "a"));
        assert_eq!(field, "b");
    }

    #[test]
    fn keeps_machine_array_initializers_raw() {
        let tokens = tokenize("BYTE ARRAY code=[$A9 $00 $60]").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Declaration(Decl::Var(var)) = &program.modules[0].items[0] else {
            panic!("expected declaration");
        };

        assert_eq!(
            var.entries[0].initializer.as_ref().unwrap().kind,
            ExprKind::Raw
        );
    }

    #[test]
    fn parses_assignments_calls_and_machine_blocks() {
        let tokens = tokenize("PROC Main() x=1 y==+1 Print(x) [$A9 $00 _Cio] RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        assert!(matches!(routine.body[0], Stmt::Assign { .. }));
        assert!(matches!(
            routine.body[1],
            Stmt::CompoundAssign {
                op: BinaryOp::Add,
                ..
            }
        ));
        assert!(matches!(routine.body[2], Stmt::Call { .. }));
        assert!(matches!(routine.body[3], Stmt::MachineBlock { .. }));
    }

    #[test]
    fn parses_colon_separated_statements() {
        let tokens = tokenize("PROC Main() x=1:y==+1:Print(x):RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        assert_eq!(routine.body.len(), 4);
        assert!(matches!(routine.body[0], Stmt::Assign { .. }));
        assert!(matches!(routine.body[1], Stmt::CompoundAssign { .. }));
        assert!(matches!(routine.body[2], Stmt::Call { .. }));
        assert!(matches!(routine.body[3], Stmt::Return(None)));
    }

    #[test]
    fn stops_routine_body_before_following_set_directive() {
        let tokens = tokenize("PROC Main() Handle() SET BUFFER=*").unwrap();
        let program = parse(&tokens).unwrap();

        assert_eq!(program.modules[0].items.len(), 2);
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        assert_eq!(routine.body.len(), 1);
        assert!(matches!(routine.body[0], Stmt::Call { .. }));
        assert!(matches!(program.modules[0].items[1], Item::Set(_)));
    }

    #[test]
    fn parses_nested_control_flow_statements() {
        let tokens =
            tokenize("PROC Main() WHILE x DO IF x THEN EXIT ELSE y=1 FI OD RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        let Stmt::While { body, .. } = &routine.body[0] else {
            panic!("expected while");
        };
        assert!(matches!(body[0], Stmt::If { .. }));
    }

    #[test]
    fn stops_if_branches_at_else_tokens() {
        let tokens =
            tokenize("PROC Main() IF x THEN y=1 ELSEIF z THEN y=2 ELSE y=3 FI RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::If {
            branches,
            else_body,
            ..
        } = &routine.body[0]
        else {
            panic!("expected if");
        };

        assert_eq!(routine.body.len(), 2);
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].body.len(), 1);
        assert_eq!(branches[1].body.len(), 1);
        assert_eq!(else_body.len(), 1);
        assert!(matches!(routine.body[1], Stmt::Return(None)));
    }

    #[test]
    fn stops_loop_body_at_od_and_continues_after_loop() {
        let tokens = tokenize("PROC Main() WHILE x DO y=1 OD z=2 RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::While { body, .. } = &routine.body[0] else {
            panic!("expected while");
        };

        assert_eq!(body.len(), 1);
        assert_eq!(routine.body.len(), 3);
        assert!(matches!(routine.body[1], Stmt::Assign { .. }));
        assert!(matches!(routine.body[2], Stmt::Return(None)));
    }

    #[test]
    fn skips_colon_before_block_terminators() {
        let tokens = tokenize("PROC Main() IF x THEN y=1:ELSE y=2:FI:RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Stmt::If {
            branches,
            else_body,
            ..
        } = &routine.body[0]
        else {
            panic!("expected if");
        };

        assert_eq!(branches[0].body.len(), 1);
        assert_eq!(else_body.len(), 1);
        assert_eq!(routine.body.len(), 2);
        assert!(matches!(routine.body[1], Stmt::Return(None)));
    }

    #[test]
    fn parses_for_and_do_until_statements() {
        let tokens =
            tokenize("PROC Main() FOR i=1 TO 3 DO DO i==+1 UNTIL i=3 OD OD RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        let Stmt::For { body, .. } = &routine.body[0] else {
            panic!("expected for");
        };
        assert!(matches!(body[0], Stmt::DoUntil { .. }));
    }

    #[test]
    fn stops_routine_body_at_next_routine() {
        let tokens = tokenize("PROC One() x=1 PROC Two() y=2 RETURN").unwrap();
        let program = parse(&tokens).unwrap();

        assert_eq!(program.modules[0].items.len(), 2);
        let Item::Routine(first) = &program.modules[0].items[0] else {
            panic!("expected first routine");
        };
        let Item::Routine(second) = &program.modules[0].items[1] else {
            panic!("expected second routine");
        };

        assert_eq!(first.body.len(), 1);
        assert_eq!(second.body.len(), 2);
    }

    #[test]
    fn parses_type_and_record_declarations() {
        let tokens =
            tokenize("TYPE Pair=[BYTE left, right CARD value] RECORD Header=[BYTE tag]").unwrap();
        let program = parse(&tokens).unwrap();
        assert_eq!(program.modules[0].items.len(), 2);
        assert!(matches!(
            program.modules[0].items[0],
            Item::Declaration(Decl::Type(_))
        ));
        assert!(matches!(
            program.modules[0].items[1],
            Item::Declaration(Decl::Record(_))
        ));
    }

    #[test]
    fn stops_array_initializer_before_next_local_declaration() {
        let tokens = tokenize("PROC fuji() BYTE ARRAY data=[1] CARD x color=1 RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        assert_eq!(routine.locals.len(), 2);
        let Decl::Var(second) = &routine.locals[1] else {
            panic!("expected var declaration");
        };
        assert_eq!(second.entries[0].name, "x");
    }

    #[test]
    fn stops_local_declarations_before_assignment_statements() {
        let tokens = tokenize(
            "BYTE FUNC GCheck(BYTE bN,x,y,bI,bD)\nBYTE bR,bK,bC\nbR=bD\nbC=bD\nRETURN(bR)",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };

        assert_eq!(routine.locals.len(), 1);
        assert!(!routine.body.is_empty());
    }

    #[test]
    fn parses_named_pointer_local_declaration_after_trailing_comma() {
        let tokens = tokenize(
            "TYPE BLOCK=[CARD size,next]\nCARD FUNC Alloc(CARD nBytes)\nBLOCK POINTER last, current,\ntarget\nlast=target\nRETURN(target)",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        let Decl::Var(local) = &routine.locals[0] else {
            panic!("expected local declaration");
        };

        assert_eq!(routine.locals.len(), 1);
        assert_eq!(local.entries.len(), 3);
        assert_eq!(local.entries[2].name, "target");
        assert!(matches!(routine.body[0], Stmt::Assign { .. }));
    }

    #[test]
    fn parses_fundamental_local_declaration_before_two_identifier_statement() {
        let tokens =
            tokenize("PROC Circle() INT Phi,Phiy,Phixy,\nx1,y1\nPhi=0\nx1=y1\nRETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[0] else {
            panic!("expected routine");
        };
        let Decl::Var(local) = &routine.locals[0] else {
            panic!("expected local declaration");
        };

        assert_eq!(routine.locals.len(), 1);
        assert_eq!(local.entries.len(), 5);
        assert_eq!(local.entries[3].name, "x1");
        assert!(matches!(routine.body[0], Stmt::Assign { .. }));
    }

    #[test]
    fn parses_named_value_local_declaration_before_two_identifier_statement() {
        let tokens = tokenize(
            "TYPE REAL=[CARD r1,r2,r3]\nPROC Demo()\nREAL x,y,z\nGraphics(1)\nz.r1=3\nRETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        let Decl::Var(local) = &routine.locals[0] else {
            panic!("expected local declaration");
        };

        assert_eq!(routine.locals.len(), 1);
        assert_eq!(local.entries.len(), 3);
        assert_eq!(local.entries[2].name, "z");
        assert!(matches!(routine.body[0], Stmt::Call { .. }));
        assert!(matches!(routine.body[1], Stmt::Assign { .. }));
    }

    #[test]
    fn parses_adjacent_machine_define_statements_after_locals() {
        let tokens = tokenize(
            "DEFINE PushAXY=\"[$48]\", PullYXA=\"[$68]\", SaveTemps=\"[$A2]\", GetTemps=\"[$A0]\"\n\
             PROC ScrollColors()\n\
             BYTE temp,i\n\
             PushAXY\n\
             SaveTemps\n\
             temp=1\n\
             Timer2=2\n\
             GetTemps\n\
             PullYXA\n\
             RETURN",
        )
        .unwrap();
        let program = parse(&tokens).unwrap();
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };

        assert_eq!(routine.locals.len(), 1);
        assert!(matches!(routine.body[0], Stmt::Call { .. }));
        assert!(matches!(routine.body[1], Stmt::Call { .. }));
        assert!(matches!(routine.body[4], Stmt::Call { .. }));
        assert!(matches!(routine.body[5], Stmt::Call { .. }));
        assert!(matches!(routine.body[6], Stmt::Return(_)));
    }
}
