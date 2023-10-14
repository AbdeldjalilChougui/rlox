//! Recursive-descent parser producing the AST used by the tree-walker.

use super::ast::{next_id, Expr, FunDecl, Lit, Stmt};
use crate::scanner::{Scanner, Token, TokenKind as T};
use std::rc::Rc;

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    pub errors: Vec<String>,
}

struct ParseError;
type PResult<X> = Result<X, ParseError>;

pub fn parse(source: &str) -> Result<Vec<Stmt>, Vec<String>> {
    let mut p = Parser { tokens: Scanner::new(source).scan_all(), current: 0, errors: Vec::new() };
    let mut stmts = Vec::new();
    while !p.check(T::Eof) {
        match p.declaration() {
            Ok(s) => stmts.push(s),
            Err(ParseError) => p.synchronize(),
        }
    }
    if p.errors.is_empty() {
        Ok(stmts)
    } else {
        Err(p.errors)
    }
}

pub fn format_error(tok: &Token, msg: &str) -> String {
    match tok.kind {
        T::Eof => format!("[line {}] Error at end: {msg}", tok.line),
        T::Error => format!("[line {}] Error: {}", tok.line, tok.lexeme),
        _ => format!("[line {}] Error at '{}': {msg}", tok.line, tok.lexeme),
    }
}

impl Parser {
    // ---- statements -------------------------------------------------------

    fn declaration(&mut self) -> PResult<Stmt> {
        if self.matches(T::Class) {
            self.class_decl()
        } else if self.matches(T::Fun) {
            Ok(Stmt::Function(self.function("function")?))
        } else if self.matches(T::Var) {
            self.var_decl()
        } else {
            self.statement()
        }
    }

    fn class_decl(&mut self) -> PResult<Stmt> {
        let name = self.consume(T::Identifier, "Expect class name.")?;
        let superclass = if self.matches(T::Less) {
            let n = self.consume(T::Identifier, "Expect superclass name.")?;
            Some(Expr::Variable { name: n, id: next_id() })
        } else {
            None
        };
        self.consume(T::LeftBrace, "Expect '{' before class body.")?;
        let mut methods = Vec::new();
        while !self.check(T::RightBrace) && !self.check(T::Eof) {
            methods.push(self.function("method")?);
        }
        self.consume(T::RightBrace, "Expect '}' after class body.")?;
        Ok(Stmt::Class { name, superclass, methods })
    }

    fn function(&mut self, kind: &str) -> PResult<Rc<FunDecl>> {
        let name = self.consume(T::Identifier, &format!("Expect {kind} name."))?;
        self.consume(T::LeftParen, &format!("Expect '(' after {kind} name."))?;
        let mut params = Vec::new();
        if !self.check(T::RightParen) {
            loop {
                if params.len() >= 255 {
                    let t = self.peek().clone();
                    self.error(&t, "Can't have more than 255 parameters.");
                }
                params.push(self.consume(T::Identifier, "Expect parameter name.")?);
                if !self.matches(T::Comma) {
                    break;
                }
            }
        }
        self.consume(T::RightParen, "Expect ')' after parameters.")?;
        self.consume(T::LeftBrace, &format!("Expect '{{' before {kind} body."))?;
        let body = self.block()?;
        Ok(Rc::new(FunDecl { name, params, body }))
    }

    fn var_decl(&mut self) -> PResult<Stmt> {
        let name = self.consume(T::Identifier, "Expect variable name.")?;
        let init = if self.matches(T::Equal) { Some(self.expression()?) } else { None };
        self.consume(T::Semicolon, "Expect ';' after variable declaration.")?;
        Ok(Stmt::Var { name, init })
    }

    fn statement(&mut self) -> PResult<Stmt> {
        if self.matches(T::Print) {
            let e = self.expression()?;
            self.consume(T::Semicolon, "Expect ';' after value.")?;
            return Ok(Stmt::Print(e));
        }
        if self.matches(T::LeftBrace) {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.matches(T::If) {
            self.consume(T::LeftParen, "Expect '(' after 'if'.")?;
            let cond = self.expression()?;
            self.consume(T::RightParen, "Expect ')' after if condition.")?;
            let then = Box::new(self.statement()?);
            let otherwise = if self.matches(T::Else) { Some(Box::new(self.statement()?)) } else { None };
            return Ok(Stmt::If { cond, then, otherwise });
        }
        if self.matches(T::While) {
            self.consume(T::LeftParen, "Expect '(' after 'while'.")?;
            let cond = self.expression()?;
            self.consume(T::RightParen, "Expect ')' after condition.")?;
            let body = Box::new(self.statement()?);
            return Ok(Stmt::While { cond, body });
        }
        if self.matches(T::For) {
            return self.for_stmt();
        }
        if self.matches(T::Return) {
            let keyword = self.previous().clone();
            let value = if self.check(T::Semicolon) { None } else { Some(self.expression()?) };
            self.consume(T::Semicolon, "Expect ';' after return value.")?;
            return Ok(Stmt::Return { keyword, value });
        }
        let e = self.expression()?;
        self.consume(T::Semicolon, "Expect ';' after expression.")?;
        Ok(Stmt::Expr(e))
    }

    /// `for` is desugared into a `while` loop inside a block.
    fn for_stmt(&mut self) -> PResult<Stmt> {
        self.consume(T::LeftParen, "Expect '(' after 'for'.")?;
        let init = if self.matches(T::Semicolon) {
            None
        } else if self.matches(T::Var) {
            Some(self.var_decl()?)
        } else {
            let e = self.expression()?;
            self.consume(T::Semicolon, "Expect ';' after expression.")?;
            Some(Stmt::Expr(e))
        };
        let cond = if self.check(T::Semicolon) { None } else { Some(self.expression()?) };
        self.consume(T::Semicolon, "Expect ';' after loop condition.")?;
        let incr = if self.check(T::RightParen) { None } else { Some(self.expression()?) };
        self.consume(T::RightParen, "Expect ')' after for clauses.")?;
        let mut body = self.statement()?;
        if let Some(incr) = incr {
            body = Stmt::Block(vec![body, Stmt::Expr(incr)]);
        }
        let cond = cond.unwrap_or(Expr::Literal(Lit::Bool(true)));
        body = Stmt::While { cond, body: Box::new(body) };
        if let Some(init) = init {
            body = Stmt::Block(vec![init, body]);
        }
        Ok(body)
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        let mut stmts = Vec::new();
        while !self.check(T::RightBrace) && !self.check(T::Eof) {
            match self.declaration() {
                Ok(s) => stmts.push(s),
                Err(ParseError) => self.synchronize(),
            }
        }
        self.consume(T::RightBrace, "Expect '}' after block.")?;
        Ok(stmts)
    }

    // ---- expressions ------------------------------------------------------

    fn expression(&mut self) -> PResult<Expr> {
        self.assignment()
    }

    fn assignment(&mut self) -> PResult<Expr> {
        let expr = self.or()?;
        if self.matches(T::Equal) {
            let equals = self.previous().clone();
            let value = Box::new(self.assignment()?);
            return match expr {
                Expr::Variable { name, .. } => Ok(Expr::Assign { name, value, id: next_id() }),
                Expr::Get { object, name } => Ok(Expr::Set { object, name, value }),
                other => {
                    self.error(&equals, "Invalid assignment target.");
                    Ok(other)
                }
            };
        }
        Ok(expr)
    }

    fn or(&mut self) -> PResult<Expr> {
        let mut e = self.and()?;
        while self.matches(T::Or) {
            let op = self.previous().clone();
            let right = self.and()?;
            e = Expr::Logical { left: Box::new(e), op, right: Box::new(right) };
        }
        Ok(e)
    }

    fn and(&mut self) -> PResult<Expr> {
        let mut e = self.binary_level(0)?;
        while self.matches(T::And) {
            let op = self.previous().clone();
            let right = self.binary_level(0)?;
            e = Expr::Logical { left: Box::new(e), op, right: Box::new(right) };
        }
        Ok(e)
    }

    /// Precedence-climbing over the left-associative binary levels:
    /// equality < comparison < term < factor.
    fn binary_level(&mut self, level: usize) -> PResult<Expr> {
        const LEVELS: [&[T]; 4] = [
            &[T::BangEqual, T::EqualEqual],
            &[T::Greater, T::GreaterEqual, T::Less, T::LessEqual],
            &[T::Minus, T::Plus],
            &[T::Slash, T::Star],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut e = self.binary_level(level + 1)?;
        while LEVELS[level].contains(&self.peek().kind) {
            let op = self.advance().clone();
            let right = self.binary_level(level + 1)?;
            e = Expr::Binary { left: Box::new(e), op, right: Box::new(right) };
        }
        Ok(e)
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.matches(T::Bang) || self.matches(T::Minus) {
            let op = self.previous().clone();
            let right = Box::new(self.unary()?);
            return Ok(Expr::Unary { op, right });
        }
        self.call()
    }

    fn call(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.matches(T::LeftParen) {
                let mut args = Vec::new();
                if !self.check(T::RightParen) {
                    loop {
                        if args.len() >= 255 {
                            let t = self.peek().clone();
                            self.error(&t, "Can't have more than 255 arguments.");
                        }
                        args.push(self.expression()?);
                        if !self.matches(T::Comma) {
                            break;
                        }
                    }
                }
                let paren = self.consume(T::RightParen, "Expect ')' after arguments.")?;
                e = Expr::Call { callee: Box::new(e), paren, args };
            } else if self.matches(T::Dot) {
                let name = self.consume(T::Identifier, "Expect property name after '.'.")?;
                e = Expr::Get { object: Box::new(e), name };
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let t = self.peek().clone();
        if t.kind != T::Eof {
            self.current += 1;
        }
        Ok(match t.kind {
            T::False => Expr::Literal(Lit::Bool(false)),
            T::True => Expr::Literal(Lit::Bool(true)),
            T::Nil => Expr::Literal(Lit::Nil),
            T::Number => Expr::Literal(Lit::Number(t.lexeme.parse().unwrap_or(0.0))),
            T::String => Expr::Literal(Lit::Str(t.lexeme[1..t.lexeme.len() - 1].into())),
            T::This => Expr::This { keyword: t, id: next_id() },
            T::Super => {
                self.consume(T::Dot, "Expect '.' after 'super'.")?;
                let method = self.consume(T::Identifier, "Expect superclass method name.")?;
                Expr::Super { keyword: t, method, id: next_id() }
            }
            T::Identifier => Expr::Variable { name: t, id: next_id() },
            T::LeftParen => {
                let e = self.expression()?;
                self.consume(T::RightParen, "Expect ')' after expression.")?;
                Expr::Grouping(Box::new(e))
            }
            _ => {
                if t.kind != T::Eof {
                    self.current -= 1;
                }
                return Err(self.error(&t, "Expect expression."));
            }
        })
    }

    // ---- helpers ----------------------------------------------------------

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    fn check(&self, k: T) -> bool {
        self.peek().kind == k
    }

    fn advance(&mut self) -> &Token {
        if !self.check(T::Eof) {
            self.current += 1;
        }
        self.previous()
    }

    fn matches(&mut self, k: T) -> bool {
        if self.check(k) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn consume(&mut self, k: T, msg: &str) -> PResult<Token> {
        if self.check(k) {
            return Ok(self.advance().clone());
        }
        let t = self.peek().clone();
        Err(self.error(&t, msg))
    }

    fn error(&mut self, tok: &Token, msg: &str) -> ParseError {
        self.errors.push(format_error(tok, msg));
        ParseError
    }

    fn synchronize(&mut self) {
        // Always make progress so we cannot loop on the same bad token.
        if !self.check(T::Eof) {
            self.current += 1;
        }
        while !self.check(T::Eof) {
            if self.previous().kind == T::Semicolon {
                return;
            }
            match self.peek().kind {
                T::Class | T::Fun | T::Var | T::For | T::If | T::While | T::Print | T::Return => return,
                _ => self.current += 1,
            }
        }
    }
}
