use crate::scanner::Token;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Unique id for expressions that refer to variables, so the resolver can
/// record their scope depth (ids stay unique across REPL lines).
pub fn next_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub enum Lit {
    Nil,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
}

#[derive(Debug)]
pub enum Expr {
    Literal(Lit),
    Grouping(Box<Expr>),
    Unary { op: Token, right: Box<Expr> },
    Binary { left: Box<Expr>, op: Token, right: Box<Expr> },
    Logical { left: Box<Expr>, op: Token, right: Box<Expr> },
    Variable { name: Token, id: usize },
    Assign { name: Token, value: Box<Expr>, id: usize },
    Call { callee: Box<Expr>, paren: Token, args: Vec<Expr> },
    Get { object: Box<Expr>, name: Token },
    Set { object: Box<Expr>, name: Token, value: Box<Expr> },
    This { keyword: Token, id: usize },
    Super { keyword: Token, method: Token, id: usize },
}

#[derive(Debug)]
pub struct FunDecl {
    pub name: Token,
    pub params: Vec<Token>,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub enum Stmt {
    Expr(Expr),
    Print(Expr),
    Var { name: Token, init: Option<Expr> },
    Block(Vec<Stmt>),
    If { cond: Expr, then: Box<Stmt>, otherwise: Option<Box<Stmt>> },
    While { cond: Expr, body: Box<Stmt> },
    Function(Rc<FunDecl>),
    Return { keyword: Token, value: Option<Expr> },
    Class { name: Token, superclass: Option<Expr>, methods: Vec<Rc<FunDecl>> },
}
