//! Static pass that resolves each local variable reference to a scope depth.

use super::ast::{Expr, FunDecl, Stmt};
use super::parser::format_error;
use crate::scanner::Token;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
enum FnKind {
    None,
    Function,
    Method,
    Initializer,
}

#[derive(Clone, Copy, PartialEq)]
enum ClassKind {
    None,
    Class,
    Subclass,
}

pub struct Resolver<'a> {
    scopes: Vec<HashMap<String, bool>>,
    locals: &'a mut HashMap<usize, usize>,
    func: FnKind,
    class: ClassKind,
    pub errors: Vec<String>,
}

impl<'a> Resolver<'a> {
    pub fn new(locals: &'a mut HashMap<usize, usize>) -> Self {
        Resolver { scopes: Vec::new(), locals, func: FnKind::None, class: ClassKind::None, errors: Vec::new() }
    }

    pub fn resolve(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn error(&mut self, tok: &Token, msg: &str) {
        self.errors.push(format_error(tok, msg));
    }

    fn begin(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn end(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &Token) {
        let dup = match self.scopes.last_mut() {
            Some(scope) => scope.insert(name.lexeme.clone(), false).is_some(),
            None => false,
        };
        if dup {
            self.error(name, "Already a variable with this name in this scope.");
        }
    }

    fn define(&mut self, name: &Token) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.lexeme.clone(), true);
        }
    }

    fn resolve_local(&mut self, id: usize, name: &str) {
        for (depth, scope) in self.scopes.iter().rev().enumerate() {
            if scope.contains_key(name) {
                self.locals.insert(id, depth);
                return;
            }
        }
        // Not found: global.
    }

    fn function(&mut self, f: &FunDecl, kind: FnKind) {
        let enclosing = std::mem::replace(&mut self.func, kind);
        self.begin();
        for p in &f.params {
            self.declare(p);
            self.define(p);
        }
        self.resolve(&f.body);
        self.end();
        self.func = enclosing;
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(b) => {
                self.begin();
                self.resolve(b);
                self.end();
            }
            Stmt::Var { name, init } => {
                self.declare(name);
                if let Some(e) = init {
                    self.expr(e);
                }
                self.define(name);
            }
            Stmt::Function(f) => {
                self.declare(&f.name);
                self.define(&f.name);
                self.function(f, FnKind::Function);
            }
            Stmt::Class { name, superclass, methods } => {
                let enclosing = std::mem::replace(&mut self.class, ClassKind::Class);
                self.declare(name);
                self.define(name);
                if let Some(sc) = superclass {
                    if let Expr::Variable { name: sn, .. } = sc {
                        if sn.lexeme == name.lexeme {
                            self.error(sn, "A class can't inherit from itself.");
                        }
                    }
                    self.class = ClassKind::Subclass;
                    self.expr(sc);
                    self.begin();
                    self.scopes.last_mut().unwrap().insert("super".into(), true);
                }
                self.begin();
                self.scopes.last_mut().unwrap().insert("this".into(), true);
                for m in methods {
                    let kind = if m.name.lexeme == "init" { FnKind::Initializer } else { FnKind::Method };
                    self.function(m, kind);
                }
                self.end();
                if superclass.is_some() {
                    self.end();
                }
                self.class = enclosing;
            }
            Stmt::Expr(e) | Stmt::Print(e) => self.expr(e),
            Stmt::If { cond, then, otherwise } => {
                self.expr(cond);
                self.stmt(then);
                if let Some(o) = otherwise {
                    self.stmt(o);
                }
            }
            Stmt::While { cond, body } => {
                self.expr(cond);
                self.stmt(body);
            }
            Stmt::Return { keyword, value } => {
                if self.func == FnKind::None {
                    self.error(keyword, "Can't return from top-level code.");
                }
                if let Some(v) = value {
                    if self.func == FnKind::Initializer {
                        self.error(keyword, "Can't return a value from an initializer.");
                    }
                    self.expr(v);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Variable { name, id } => {
                if self.scopes.last().and_then(|s| s.get(&name.lexeme)) == Some(&false) {
                    self.error(name, "Can't read local variable in its own initializer.");
                }
                self.resolve_local(*id, &name.lexeme);
            }
            Expr::Assign { name, value, id } => {
                self.expr(value);
                self.resolve_local(*id, &name.lexeme);
            }
            Expr::This { keyword, id } => {
                if self.class == ClassKind::None {
                    self.error(keyword, "Can't use 'this' outside of a class.");
                } else {
                    self.resolve_local(*id, "this");
                }
            }
            Expr::Super { keyword, id, .. } => {
                match self.class {
                    ClassKind::None => self.error(keyword, "Can't use 'super' outside of a class."),
                    ClassKind::Class => self.error(keyword, "Can't use 'super' in a class with no superclass."),
                    ClassKind::Subclass => {}
                }
                self.resolve_local(*id, "super");
            }
            Expr::Literal(_) => {}
            Expr::Grouping(e) | Expr::Unary { right: e, .. } => self.expr(e),
            Expr::Binary { left, right, .. } | Expr::Logical { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Get { object, .. } => self.expr(object),
            Expr::Set { object, value, .. } => {
                self.expr(value);
                self.expr(object);
            }
        }
    }
}
