//! Single-pass Pratt compiler from tokens straight to bytecode ("clox" style).

use super::value::{Function, Op, Value};
use crate::scanner::{Scanner, Token, TokenKind as T};
use crate::tree::parser::format_error;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prec {
    None,
    Assignment,
    Or,
    And,
    Equality,
    Comparison,
    Term,
    Factor,
    Unary,
    Call,
    Primary,
}

impl Prec {
    fn next(self) -> Prec {
        use Prec::*;
        match self {
            None => Assignment,
            Assignment => Or,
            Or => And,
            And => Equality,
            Equality => Comparison,
            Comparison => Term,
            Term => Factor,
            Factor => Unary,
            Unary => Call,
            Call | Primary => Primary,
        }
    }
}

fn infix_prec(k: T) -> Prec {
    match k {
        T::Minus | T::Plus => Prec::Term,
        T::Slash | T::Star => Prec::Factor,
        T::BangEqual | T::EqualEqual => Prec::Equality,
        T::Greater | T::GreaterEqual | T::Less | T::LessEqual => Prec::Comparison,
        T::And => Prec::And,
        T::Or => Prec::Or,
        T::LeftParen | T::Dot => Prec::Call,
        _ => Prec::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FnKind {
    Script,
    Function,
    Method,
    Initializer,
}

struct Local {
    name: String,
    /// -1 while declared but not yet initialized.
    depth: i32,
    captured: bool,
}

struct UpvalueRef {
    index: u8,
    is_local: bool,
}

struct FnState {
    function: Function,
    kind: FnKind,
    locals: Vec<Local>,
    upvalues: Vec<UpvalueRef>,
    scope_depth: i32,
}

impl FnState {
    fn new(kind: FnKind, name: Option<Rc<str>>) -> FnState {
        let slot0 = if matches!(kind, FnKind::Method | FnKind::Initializer) { "this" } else { "" };
        FnState {
            function: Function { name, ..Default::default() },
            kind,
            locals: vec![Local { name: slot0.into(), depth: 0, captured: false }],
            upvalues: Vec::new(),
            scope_depth: 0,
        }
    }
}

pub struct Compiler<'s> {
    scanner: Scanner<'s>,
    current: Token,
    previous: Token,
    panic_mode: bool,
    errors: Vec<String>,
    states: Vec<FnState>,
    /// One entry per enclosing class; `true` if it has a superclass.
    classes: Vec<bool>,
}

pub fn compile(source: &str) -> Result<Function, Vec<String>> {
    let dummy = Token { kind: T::Eof, lexeme: String::new(), line: 0 };
    let mut c = Compiler {
        scanner: Scanner::new(source),
        current: dummy.clone(),
        previous: dummy,
        panic_mode: false,
        errors: Vec::new(),
        states: vec![FnState::new(FnKind::Script, None)],
        classes: Vec::new(),
    };
    c.advance();
    while !c.matches(T::Eof) {
        c.declaration();
    }
    let f = c.end_function().0;
    if c.errors.is_empty() {
        Ok(f)
    } else {
        Err(c.errors)
    }
}

impl<'s> Compiler<'s> {
    // ---- token plumbing ---------------------------------------------------

    fn advance(&mut self) {
        self.previous = std::mem::replace(&mut self.current, self.scanner.next_token());
        while self.current.kind == T::Error {
            let msg = self.current.lexeme.clone();
            self.error_at_current(&msg);
            self.current = self.scanner.next_token();
        }
    }

    fn check(&self, k: T) -> bool {
        self.current.kind == k
    }

    fn matches(&mut self, k: T) -> bool {
        if !self.check(k) {
            return false;
        }
        self.advance();
        true
    }

    fn consume(&mut self, k: T, msg: &str) {
        if self.check(k) {
            self.advance();
        } else {
            self.error_at_current(msg);
        }
    }

    fn error_at(&mut self, tok: Token, msg: &str) {
        if self.panic_mode {
            return;
        }
        self.panic_mode = true;
        self.errors.push(format_error(&tok, msg));
    }

    fn error(&mut self, msg: &str) {
        self.error_at(self.previous.clone(), msg);
    }

    fn error_at_current(&mut self, msg: &str) {
        self.error_at(self.current.clone(), msg);
    }

    fn synchronize(&mut self) {
        self.panic_mode = false;
        while self.current.kind != T::Eof {
            if self.previous.kind == T::Semicolon {
                return;
            }
            match self.current.kind {
                T::Class | T::Fun | T::Var | T::For | T::If | T::While | T::Print | T::Return => return,
                _ => self.advance(),
            }
        }
    }

    // ---- emitting ---------------------------------------------------------

    fn state(&mut self) -> &mut FnState {
        self.states.last_mut().unwrap()
    }

    fn emit(&mut self, byte: u8) {
        let line = self.previous.line;
        self.state().function.chunk.write(byte, line);
    }

    fn emit_op(&mut self, op: Op) {
        self.emit(op as u8);
    }

    fn emit_op_arg(&mut self, op: Op, arg: u8) {
        self.emit(op as u8);
        self.emit(arg);
    }

    fn make_constant(&mut self, v: Value) -> u8 {
        let consts = &mut self.state().function.chunk.constants;
        if let Value::Str(s) = &v {
            if let Some(i) = consts.iter().position(|c| matches!(c, Value::Str(x) if x == s)) {
                return i as u8;
            }
        }
        consts.push(v);
        let idx = consts.len() - 1;
        if idx > u8::MAX as usize {
            self.error("Too many constants in one chunk.");
            return 0;
        }
        idx as u8
    }

    fn emit_constant(&mut self, v: Value) {
        let c = self.make_constant(v);
        self.emit_op_arg(Op::Constant, c);
    }

    fn emit_jump(&mut self, op: Op) -> usize {
        self.emit_op(op);
        self.emit(0xff);
        self.emit(0xff);
        self.state().function.chunk.code.len() - 2
    }

    fn patch_jump(&mut self, at: usize) {
        let jump = self.state().function.chunk.code.len() - at - 2;
        if jump > u16::MAX as usize {
            self.error("Too much code to jump over.");
        }
        let code = &mut self.state().function.chunk.code;
        code[at] = (jump >> 8) as u8;
        code[at + 1] = jump as u8;
    }

    fn emit_loop(&mut self, start: usize) {
        self.emit_op(Op::Loop);
        let offset = self.state().function.chunk.code.len() - start + 2;
        if offset > u16::MAX as usize {
            self.error("Loop body too large.");
        }
        self.emit((offset >> 8) as u8);
        self.emit(offset as u8);
    }

    fn emit_return(&mut self) {
        if self.state().kind == FnKind::Initializer {
            self.emit_op_arg(Op::GetLocal, 0);
        } else {
            self.emit_op(Op::Nil);
        }
        self.emit_op(Op::Return);
    }

    fn end_function(&mut self) -> (Function, Vec<UpvalueRef>) {
        self.emit_return();
        let mut st = self.states.pop().unwrap();
        st.function.upvalue_count = st.upvalues.len();
        (st.function, st.upvalues)
    }

    // ---- scopes and variables ---------------------------------------------

    fn begin_scope(&mut self) {
        self.state().scope_depth += 1;
    }

    fn end_scope(&mut self) {
        self.state().scope_depth -= 1;
        loop {
            let st = self.state();
            let Some(local) = st.locals.last() else { break };
            if local.depth <= st.scope_depth {
                break;
            }
            let captured = local.captured;
            st.locals.pop();
            self.emit_op(if captured { Op::CloseUpvalue } else { Op::Pop });
        }
    }

    fn identifier_constant(&mut self, name: &str) -> u8 {
        self.make_constant(Value::Str(name.into()))
    }

    fn add_local(&mut self, name: String) {
        if self.state().locals.len() > u8::MAX as usize {
            self.error("Too many local variables in function.");
            return;
        }
        self.state().locals.push(Local { name, depth: -1, captured: false });
    }

    fn declare_variable(&mut self) {
        let st = self.states.last().unwrap();
        if st.scope_depth == 0 {
            return;
        }
        let name = self.previous.lexeme.clone();
        let dup = st
            .locals
            .iter()
            .rev()
            .take_while(|l| l.depth == -1 || l.depth >= st.scope_depth)
            .any(|l| l.name == name);
        if dup {
            self.error("Already a variable with this name in this scope.");
        }
        self.add_local(name);
    }

    fn parse_variable(&mut self, msg: &str) -> u8 {
        self.consume(T::Identifier, msg);
        self.declare_variable();
        if self.state().scope_depth > 0 {
            return 0;
        }
        let name = self.previous.lexeme.clone();
        self.identifier_constant(&name)
    }

    fn mark_initialized(&mut self) {
        let st = self.state();
        if st.scope_depth == 0 {
            return;
        }
        let d = st.scope_depth;
        st.locals.last_mut().unwrap().depth = d;
    }

    fn define_variable(&mut self, global: u8) {
        if self.state().scope_depth > 0 {
            self.mark_initialized();
            return;
        }
        self.emit_op_arg(Op::DefineGlobal, global);
    }

    fn resolve_local(&mut self, depth: usize, name: &str) -> Option<u8> {
        let (idx, uninit) = {
            let st = &self.states[depth];
            let i = st.locals.iter().rposition(|l| l.name == name)?;
            (i, st.locals[i].depth == -1)
        };
        if uninit {
            self.error("Can't read local variable in its own initializer.");
        }
        Some(idx as u8)
    }

    fn add_upvalue(&mut self, depth: usize, index: u8, is_local: bool) -> u8 {
        let ups = &mut self.states[depth].upvalues;
        if let Some(i) = ups.iter().position(|u| u.index == index && u.is_local == is_local) {
            return i as u8;
        }
        if ups.len() > u8::MAX as usize {
            self.error("Too many closure variables in function.");
            return 0;
        }
        ups.push(UpvalueRef { index, is_local });
        (ups.len() - 1) as u8
    }

    fn resolve_upvalue(&mut self, depth: usize, name: &str) -> Option<u8> {
        if depth == 0 {
            return None;
        }
        if let Some(local) = self.resolve_local(depth - 1, name) {
            self.states[depth - 1].locals[local as usize].captured = true;
            return Some(self.add_upvalue(depth, local, true));
        }
        let up = self.resolve_upvalue(depth - 1, name)?;
        Some(self.add_upvalue(depth, up, false))
    }

    fn named_variable(&mut self, name: &str, can_assign: bool) {
        let depth = self.states.len() - 1;
        let (get, set, arg) = if let Some(i) = self.resolve_local(depth, name) {
            (Op::GetLocal, Op::SetLocal, i)
        } else if let Some(i) = self.resolve_upvalue(depth, name) {
            (Op::GetUpvalue, Op::SetUpvalue, i)
        } else {
            let c = self.identifier_constant(name);
            (Op::GetGlobal, Op::SetGlobal, c)
        };
        if can_assign && self.matches(T::Equal) {
            self.expression();
            self.emit_op_arg(set, arg);
        } else {
            self.emit_op_arg(get, arg);
        }
    }

    // ---- declarations and statements --------------------------------------

    fn declaration(&mut self) {
        if self.matches(T::Class) {
            self.class_declaration();
        } else if self.matches(T::Fun) {
            let global = self.parse_variable("Expect function name.");
            self.mark_initialized();
            self.function(FnKind::Function);
            self.define_variable(global);
        } else if self.matches(T::Var) {
            self.var_declaration();
        } else {
            self.statement();
        }
        if self.panic_mode {
            self.synchronize();
        }
    }

    fn class_declaration(&mut self) {
        self.consume(T::Identifier, "Expect class name.");
        let class_name = self.previous.lexeme.clone();
        let name_const = self.identifier_constant(&class_name);
        self.declare_variable();
        self.emit_op_arg(Op::Class, name_const);
        self.define_variable(name_const);
        self.classes.push(false);

        if self.matches(T::Less) {
            self.consume(T::Identifier, "Expect superclass name.");
            let super_name = self.previous.lexeme.clone();
            self.named_variable(&super_name, false);
            if super_name == class_name {
                self.error("A class can't inherit from itself.");
            }
            self.begin_scope();
            self.add_local("super".into());
            self.define_variable(0);
            self.named_variable(&class_name, false);
            self.emit_op(Op::Inherit);
            *self.classes.last_mut().unwrap() = true;
        }

        self.named_variable(&class_name, false);
        self.consume(T::LeftBrace, "Expect '{' before class body.");
        while !self.check(T::RightBrace) && !self.check(T::Eof) {
            self.consume(T::Identifier, "Expect method name.");
            let name = self.previous.lexeme.clone();
            let c = self.identifier_constant(&name);
            let kind = if name == "init" { FnKind::Initializer } else { FnKind::Method };
            self.function(kind);
            self.emit_op_arg(Op::Method, c);
        }
        self.consume(T::RightBrace, "Expect '}' after class body.");
        self.emit_op(Op::Pop);
        if self.classes.pop().unwrap() {
            self.end_scope();
        }
    }

    fn function(&mut self, kind: FnKind) {
        let name: Rc<str> = self.previous.lexeme.as_str().into();
        self.states.push(FnState::new(kind, Some(name)));
        self.begin_scope();
        self.consume(T::LeftParen, "Expect '(' after function name.");
        if !self.check(T::RightParen) {
            loop {
                self.state().function.arity += 1;
                if self.state().function.arity > 255 {
                    self.error_at_current("Can't have more than 255 parameters.");
                }
                let c = self.parse_variable("Expect parameter name.");
                self.define_variable(c);
                if !self.matches(T::Comma) {
                    break;
                }
            }
        }
        self.consume(T::RightParen, "Expect ')' after parameters.");
        self.consume(T::LeftBrace, "Expect '{' before function body.");
        self.block();
        let (function, upvalues) = self.end_function();
        let c = self.make_constant(Value::Function(Rc::new(function)));
        self.emit_op_arg(Op::Closure, c);
        for u in upvalues {
            self.emit(u.is_local as u8);
            self.emit(u.index);
        }
    }

    fn var_declaration(&mut self) {
        let global = self.parse_variable("Expect variable name.");
        if self.matches(T::Equal) {
            self.expression();
        } else {
            self.emit_op(Op::Nil);
        }
        self.consume(T::Semicolon, "Expect ';' after variable declaration.");
        self.define_variable(global);
    }

    fn statement(&mut self) {
        if self.matches(T::Print) {
            self.expression();
            self.consume(T::Semicolon, "Expect ';' after value.");
            self.emit_op(Op::Print);
        } else if self.matches(T::For) {
            self.for_statement();
        } else if self.matches(T::If) {
            self.if_statement();
        } else if self.matches(T::Return) {
            self.return_statement();
        } else if self.matches(T::While) {
            self.while_statement();
        } else if self.matches(T::LeftBrace) {
            self.begin_scope();
            self.block();
            self.end_scope();
        } else {
            self.expression();
            self.consume(T::Semicolon, "Expect ';' after expression.");
            self.emit_op(Op::Pop);
        }
    }

    fn block(&mut self) {
        while !self.check(T::RightBrace) && !self.check(T::Eof) {
            self.declaration();
        }
        self.consume(T::RightBrace, "Expect '}' after block.");
    }

    fn if_statement(&mut self) {
        self.consume(T::LeftParen, "Expect '(' after 'if'.");
        self.expression();
        self.consume(T::RightParen, "Expect ')' after condition.");
        let then_jump = self.emit_jump(Op::JumpIfFalse);
        self.emit_op(Op::Pop);
        self.statement();
        let else_jump = self.emit_jump(Op::Jump);
        self.patch_jump(then_jump);
        self.emit_op(Op::Pop);
        if self.matches(T::Else) {
            self.statement();
        }
        self.patch_jump(else_jump);
    }

    fn while_statement(&mut self) {
        let start = self.state().function.chunk.code.len();
        self.consume(T::LeftParen, "Expect '(' after 'while'.");
        self.expression();
        self.consume(T::RightParen, "Expect ')' after condition.");
        let exit = self.emit_jump(Op::JumpIfFalse);
        self.emit_op(Op::Pop);
        self.statement();
        self.emit_loop(start);
        self.patch_jump(exit);
        self.emit_op(Op::Pop);
    }

    fn for_statement(&mut self) {
        self.begin_scope();
        self.consume(T::LeftParen, "Expect '(' after 'for'.");
        if self.matches(T::Semicolon) {
        } else if self.matches(T::Var) {
            self.var_declaration();
        } else {
            self.expression();
            self.consume(T::Semicolon, "Expect ';' after expression.");
            self.emit_op(Op::Pop);
        }
        let mut loop_start = self.state().function.chunk.code.len();
        let mut exit = None;
        if !self.matches(T::Semicolon) {
            self.expression();
            self.consume(T::Semicolon, "Expect ';' after loop condition.");
            exit = Some(self.emit_jump(Op::JumpIfFalse));
            self.emit_op(Op::Pop);
        }
        if !self.matches(T::RightParen) {
            let body_jump = self.emit_jump(Op::Jump);
            let incr_start = self.state().function.chunk.code.len();
            self.expression();
            self.emit_op(Op::Pop);
            self.consume(T::RightParen, "Expect ')' after for clauses.");
            self.emit_loop(loop_start);
            loop_start = incr_start;
            self.patch_jump(body_jump);
        }
        self.statement();
        self.emit_loop(loop_start);
        if let Some(exit) = exit {
            self.patch_jump(exit);
            self.emit_op(Op::Pop);
        }
        self.end_scope();
    }

    fn return_statement(&mut self) {
        if self.state().kind == FnKind::Script {
            self.error("Can't return from top-level code.");
        }
        if self.matches(T::Semicolon) {
            self.emit_return();
        } else {
            if self.state().kind == FnKind::Initializer {
                self.error("Can't return a value from an initializer.");
            }
            self.expression();
            self.consume(T::Semicolon, "Expect ';' after return value.");
            self.emit_op(Op::Return);
        }
    }

    // ---- expressions ------------------------------------------------------

    fn expression(&mut self) {
        self.parse_precedence(Prec::Assignment);
    }

    fn parse_precedence(&mut self, prec: Prec) {
        self.advance();
        let can_assign = prec <= Prec::Assignment;
        if !self.prefix(self.previous.kind, can_assign) {
            self.error("Expect expression.");
            return;
        }
        while prec <= infix_prec(self.current.kind) {
            self.advance();
            self.infix(self.previous.kind, can_assign);
        }
        if can_assign && self.matches(T::Equal) {
            self.error("Invalid assignment target.");
        }
    }

    fn prefix(&mut self, kind: T, can_assign: bool) -> bool {
        match kind {
            T::LeftParen => {
                self.expression();
                self.consume(T::RightParen, "Expect ')' after expression.");
            }
            T::Minus | T::Bang => {
                self.parse_precedence(Prec::Unary);
                self.emit_op(if kind == T::Minus { Op::Negate } else { Op::Not });
            }
            T::Number => {
                let n = self.previous.lexeme.parse().unwrap_or(0.0);
                self.emit_constant(Value::Number(n));
            }
            T::String => {
                let l = &self.previous.lexeme;
                let s: Rc<str> = l[1..l.len() - 1].into();
                self.emit_constant(Value::Str(s));
            }
            T::Nil => self.emit_op(Op::Nil),
            T::True => self.emit_op(Op::True),
            T::False => self.emit_op(Op::False),
            T::Identifier => {
                let name = self.previous.lexeme.clone();
                self.named_variable(&name, can_assign);
            }
            T::This => {
                if self.classes.is_empty() {
                    self.error("Can't use 'this' outside of a class.");
                } else {
                    self.named_variable("this", false);
                }
            }
            T::Super => self.super_(),
            _ => return false,
        }
        true
    }

    fn super_(&mut self) {
        match self.classes.last() {
            None => self.error("Can't use 'super' outside of a class."),
            Some(false) => self.error("Can't use 'super' in a class with no superclass."),
            Some(true) => {}
        }
        self.consume(T::Dot, "Expect '.' after 'super'.");
        self.consume(T::Identifier, "Expect superclass method name.");
        let name = self.previous.lexeme.clone();
        let c = self.identifier_constant(&name);
        self.named_variable("this", false);
        if self.matches(T::LeftParen) {
            let argc = self.argument_list();
            self.named_variable("super", false);
            self.emit_op_arg(Op::SuperInvoke, c);
            self.emit(argc);
        } else {
            self.named_variable("super", false);
            self.emit_op_arg(Op::GetSuper, c);
        }
    }

    fn infix(&mut self, kind: T, can_assign: bool) {
        match kind {
            T::And => {
                let end = self.emit_jump(Op::JumpIfFalse);
                self.emit_op(Op::Pop);
                self.parse_precedence(Prec::And);
                self.patch_jump(end);
            }
            T::Or => {
                let else_jump = self.emit_jump(Op::JumpIfFalse);
                let end = self.emit_jump(Op::Jump);
                self.patch_jump(else_jump);
                self.emit_op(Op::Pop);
                self.parse_precedence(Prec::Or);
                self.patch_jump(end);
            }
            T::LeftParen => {
                let argc = self.argument_list();
                self.emit_op_arg(Op::Call, argc);
            }
            T::Dot => {
                self.consume(T::Identifier, "Expect property name after '.'.");
                let name = self.previous.lexeme.clone();
                let c = self.identifier_constant(&name);
                if can_assign && self.matches(T::Equal) {
                    self.expression();
                    self.emit_op_arg(Op::SetProperty, c);
                } else if self.matches(T::LeftParen) {
                    let argc = self.argument_list();
                    self.emit_op_arg(Op::Invoke, c);
                    self.emit(argc);
                } else {
                    self.emit_op_arg(Op::GetProperty, c);
                }
            }
            _ => {
                self.parse_precedence(infix_prec(kind).next());
                match kind {
                    T::BangEqual => {
                        self.emit_op(Op::Equal);
                        self.emit_op(Op::Not);
                    }
                    T::EqualEqual => self.emit_op(Op::Equal),
                    T::Greater => self.emit_op(Op::Greater),
                    T::GreaterEqual => {
                        self.emit_op(Op::Less);
                        self.emit_op(Op::Not);
                    }
                    T::Less => self.emit_op(Op::Less),
                    T::LessEqual => {
                        self.emit_op(Op::Greater);
                        self.emit_op(Op::Not);
                    }
                    T::Plus => self.emit_op(Op::Add),
                    T::Minus => self.emit_op(Op::Subtract),
                    T::Star => self.emit_op(Op::Multiply),
                    T::Slash => self.emit_op(Op::Divide),
                    _ => unreachable!(),
                }
            }
        }
    }

    fn argument_list(&mut self) -> u8 {
        let mut argc = 0usize;
        if !self.check(T::RightParen) {
            loop {
                self.expression();
                if argc == 255 {
                    self.error("Can't have more than 255 arguments.");
                }
                argc += 1;
                if !self.matches(T::Comma) {
                    break;
                }
            }
        }
        self.consume(T::RightParen, "Expect ')' after arguments.");
        argc.min(255) as u8
    }
}
