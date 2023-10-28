//! Tree-walking evaluator.

use super::ast::{Expr, FunDecl, Lit, Stmt};
use super::parser;
use super::resolver::Resolver;
use crate::scanner::{Token, TokenKind as T};
use crate::LoxError;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::rc::Rc;

const MAX_DEPTH: usize = 1000;

#[derive(Default)]
pub struct EnvData {
    values: HashMap<String, Value>,
    enclosing: Option<Env>,
}
pub type Env = Rc<RefCell<EnvData>>;

fn new_env(enclosing: Option<Env>) -> Env {
    Rc::new(RefCell::new(EnvData { values: HashMap::new(), enclosing }))
}

fn ancestor(env: &Env, depth: usize) -> Env {
    let mut e = env.clone();
    for _ in 0..depth {
        let next = e.borrow().enclosing.clone().expect("resolver depth out of range");
        e = next;
    }
    e
}

pub struct LoxFunction {
    decl: Rc<FunDecl>,
    closure: Env,
    is_init: bool,
}

impl LoxFunction {
    fn bind(&self, instance: Value) -> Rc<LoxFunction> {
        let env = new_env(Some(self.closure.clone()));
        env.borrow_mut().values.insert("this".into(), instance);
        Rc::new(LoxFunction { decl: self.decl.clone(), closure: env, is_init: self.is_init })
    }
}

pub struct LoxClass {
    name: String,
    superclass: Option<Rc<LoxClass>>,
    methods: HashMap<String, Rc<LoxFunction>>,
}

impl LoxClass {
    fn find_method(&self, name: &str) -> Option<Rc<LoxFunction>> {
        self.methods
            .get(name)
            .cloned()
            .or_else(|| self.superclass.as_ref().and_then(|s| s.find_method(name)))
    }
}

pub struct Instance {
    class: Rc<LoxClass>,
    fields: HashMap<String, Value>,
}

pub struct Native {
    name: &'static str,
    arity: usize,
    f: fn(&[Value]) -> Value,
}

#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    Fun(Rc<LoxFunction>),
    Native(Rc<Native>),
    Class(Rc<LoxClass>),
    Instance(Rc<RefCell<Instance>>),
}

impl Value {
    fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Fun(a), Value::Fun(b)) => Rc::ptr_eq(a, b),
            (Value::Native(a), Value::Native(b)) => Rc::ptr_eq(a, b),
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => f.write_str("nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(n) => f.write_str(&crate::format_number(*n)),
            Value::Str(s) => f.write_str(s),
            Value::Fun(func) => write!(f, "<fn {}>", func.decl.name.lexeme),
            Value::Native(n) => write!(f, "<native fn {}>", n.name),
            Value::Class(c) => f.write_str(&c.name),
            Value::Instance(i) => write!(f, "{} instance", i.borrow().class.name),
        }
    }
}

/// Non-local exits while executing statements.
enum Unwind {
    Return(Value),
    Error(String),
}

type Exec<X> = Result<X, Unwind>;

fn rt_error<X>(tok: &Token, msg: impl fmt::Display) -> Exec<X> {
    Err(Unwind::Error(format!("{msg}\n[line {}]", tok.line)))
}

pub struct Interpreter {
    globals: Env,
    env: Env,
    locals: HashMap<usize, usize>,
    out: Box<dyn Write>,
    depth: usize,
}

impl Interpreter {
    pub fn new(out: Box<dyn Write>) -> Self {
        let globals = new_env(None);
        globals.borrow_mut().values.insert(
            "clock".into(),
            Value::Native(Rc::new(Native { name: "clock", arity: 0, f: |_| Value::Number(crate::clock_seconds()) })),
        );
        Interpreter { env: globals.clone(), globals, locals: HashMap::new(), out, depth: 0 }
    }

    pub fn run(&mut self, source: &str) -> Result<(), LoxError> {
        let stmts = parser::parse(source).map_err(LoxError::Compile)?;
        let mut r = Resolver::new(&mut self.locals);
        r.resolve(&stmts);
        if !r.errors.is_empty() {
            return Err(LoxError::Compile(r.errors));
        }
        for s in &stmts {
            match self.exec(s) {
                Ok(()) => {}
                Err(Unwind::Error(e)) => {
                    self.env = self.globals.clone();
                    self.depth = 0;
                    let _ = self.out.flush();
                    return Err(LoxError::Runtime(e));
                }
                Err(Unwind::Return(_)) => unreachable!("resolver rejects top-level return"),
            }
        }
        let _ = self.out.flush();
        Ok(())
    }

    fn exec(&mut self, s: &Stmt) -> Exec<()> {
        match s {
            Stmt::Expr(e) => {
                self.eval(e)?;
            }
            Stmt::Print(e) => {
                let v = self.eval(e)?;
                let _ = writeln!(self.out, "{v}");
            }
            Stmt::Var { name, init } => {
                let v = match init {
                    Some(e) => self.eval(e)?,
                    None => Value::Nil,
                };
                self.env.borrow_mut().values.insert(name.lexeme.clone(), v);
            }
            Stmt::Block(stmts) => {
                let env = new_env(Some(self.env.clone()));
                self.exec_block(stmts, env)?;
            }
            Stmt::If { cond, then, otherwise } => {
                if self.eval(cond)?.truthy() {
                    self.exec(then)?;
                } else if let Some(o) = otherwise {
                    self.exec(o)?;
                }
            }
            Stmt::While { cond, body } => {
                while self.eval(cond)?.truthy() {
                    self.exec(body)?;
                }
            }
            Stmt::Function(decl) => {
                let f = LoxFunction { decl: decl.clone(), closure: self.env.clone(), is_init: false };
                self.env.borrow_mut().values.insert(decl.name.lexeme.clone(), Value::Fun(Rc::new(f)));
            }
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval(e)?,
                    None => Value::Nil,
                };
                return Err(Unwind::Return(v));
            }
            Stmt::Class { name, superclass, methods } => {
                let sc = match superclass {
                    Some(e) => match self.eval(e)? {
                        Value::Class(c) => Some(c),
                        _ => {
                            let Expr::Variable { name, .. } = e else { unreachable!() };
                            return rt_error(name, "Superclass must be a class.");
                        }
                    },
                    None => None,
                };
                self.env.borrow_mut().values.insert(name.lexeme.clone(), Value::Nil);
                let method_env = match &sc {
                    Some(c) => {
                        let env = new_env(Some(self.env.clone()));
                        env.borrow_mut().values.insert("super".into(), Value::Class(c.clone()));
                        env
                    }
                    None => self.env.clone(),
                };
                let methods = methods
                    .iter()
                    .map(|m| {
                        let f = LoxFunction {
                            decl: m.clone(),
                            closure: method_env.clone(),
                            is_init: m.name.lexeme == "init",
                        };
                        (m.name.lexeme.clone(), Rc::new(f))
                    })
                    .collect();
                let class = LoxClass { name: name.lexeme.clone(), superclass: sc, methods };
                self.env.borrow_mut().values.insert(name.lexeme.clone(), Value::Class(Rc::new(class)));
            }
        }
        Ok(())
    }

    fn exec_block(&mut self, stmts: &[Stmt], env: Env) -> Exec<()> {
        let prev = std::mem::replace(&mut self.env, env);
        let mut result = Ok(());
        for s in stmts {
            result = self.exec(s);
            if result.is_err() {
                break;
            }
        }
        self.env = prev;
        result
    }

    fn lookup(&self, name: &Token, id: usize) -> Exec<Value> {
        let found = match self.locals.get(&id) {
            Some(&d) => ancestor(&self.env, d).borrow().values.get(&name.lexeme).cloned(),
            None => self.globals.borrow().values.get(&name.lexeme).cloned(),
        };
        match found {
            Some(v) => Ok(v),
            None => rt_error(name, format!("Undefined variable '{}'.", name.lexeme)),
        }
    }

    fn eval(&mut self, e: &Expr) -> Exec<Value> {
        Ok(match e {
            Expr::Literal(l) => match l {
                Lit::Nil => Value::Nil,
                Lit::Bool(b) => Value::Bool(*b),
                Lit::Number(n) => Value::Number(*n),
                Lit::Str(s) => Value::Str(s.clone()),
            },
            Expr::Grouping(e) => self.eval(e)?,
            Expr::Unary { op, right } => {
                let r = self.eval(right)?;
                match op.kind {
                    T::Bang => Value::Bool(!r.truthy()),
                    _ => match r {
                        Value::Number(n) => Value::Number(-n),
                        _ => return rt_error(op, "Operand must be a number."),
                    },
                }
            }
            Expr::Binary { left, op, right } => {
                let l = self.eval(left)?;
                let r = self.eval(right)?;
                binary(op, l, r)?
            }
            Expr::Logical { left, op, right } => {
                let l = self.eval(left)?;
                let short = if op.kind == T::Or { l.truthy() } else { !l.truthy() };
                if short {
                    l
                } else {
                    self.eval(right)?
                }
            }
            Expr::Variable { name, id } => self.lookup(name, *id)?,
            Expr::Assign { name, value, id } => {
                let v = self.eval(value)?;
                let env = match self.locals.get(id) {
                    Some(&d) => ancestor(&self.env, d),
                    None => self.globals.clone(),
                };
                let mut env = env.borrow_mut();
                match env.values.get_mut(&name.lexeme) {
                    Some(slot) => *slot = v.clone(),
                    None => return rt_error(name, format!("Undefined variable '{}'.", name.lexeme)),
                }
                v
            }
            Expr::Call { callee, paren, args } => {
                let c = self.eval(callee)?;
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.eval(a)?);
                }
                self.call(c, argv, paren)?
            }
            Expr::Get { object, name } => match self.eval(object)? {
                Value::Instance(inst) => {
                    if let Some(v) = inst.borrow().fields.get(&name.lexeme) {
                        return Ok(v.clone());
                    }
                    let class = inst.borrow().class.clone();
                    match class.find_method(&name.lexeme) {
                        Some(m) => Value::Fun(m.bind(Value::Instance(inst))),
                        None => return rt_error(name, format!("Undefined property '{}'.", name.lexeme)),
                    }
                }
                _ => return rt_error(name, "Only instances have properties."),
            },
            Expr::Set { object, name, value } => match self.eval(object)? {
                Value::Instance(inst) => {
                    let v = self.eval(value)?;
                    inst.borrow_mut().fields.insert(name.lexeme.clone(), v.clone());
                    v
                }
                _ => return rt_error(name, "Only instances have fields."),
            },
            Expr::This { keyword, id } => self.lookup(keyword, *id)?,
            Expr::Super { method, id, .. } => {
                let depth = self.locals[id];
                let env = ancestor(&self.env, depth);
                let Some(Value::Class(sc)) = env.borrow().values.get("super").cloned() else {
                    unreachable!("resolver guarantees 'super' is bound")
                };
                let this = ancestor(&self.env, depth - 1).borrow().values.get("this").cloned().unwrap();
                match sc.find_method(&method.lexeme) {
                    Some(m) => Value::Fun(m.bind(this)),
                    None => return rt_error(method, format!("Undefined property '{}'.", method.lexeme)),
                }
            }
        })
    }

    fn call(&mut self, callee: Value, args: Vec<Value>, paren: &Token) -> Exec<Value> {
        let check_arity = |n: usize| -> Exec<()> {
            if n != args.len() {
                rt_error(paren, format!("Expected {n} arguments but got {}.", args.len()))
            } else {
                Ok(())
            }
        };
        match callee {
            Value::Fun(f) => {
                check_arity(f.decl.params.len())?;
                self.call_function(&f, args, paren)
            }
            Value::Native(n) => {
                check_arity(n.arity)?;
                Ok((n.f)(&args))
            }
            Value::Class(c) => {
                let inst = Value::Instance(Rc::new(RefCell::new(Instance { class: c.clone(), fields: HashMap::new() })));
                match c.find_method("init") {
                    Some(init) => {
                        check_arity(init.decl.params.len())?;
                        self.call_function(&init.bind(inst.clone()), args, paren)?;
                    }
                    None => check_arity(0)?,
                }
                Ok(inst)
            }
            _ => rt_error(paren, "Can only call functions and classes."),
        }
    }

    fn call_function(&mut self, f: &LoxFunction, args: Vec<Value>, paren: &Token) -> Exec<Value> {
        if self.depth >= MAX_DEPTH {
            return rt_error(paren, "Stack overflow.");
        }
        let env = new_env(Some(f.closure.clone()));
        {
            let mut e = env.borrow_mut();
            for (p, a) in f.decl.params.iter().zip(args) {
                e.values.insert(p.lexeme.clone(), a);
            }
        }
        self.depth += 1;
        let r = self.exec_block(&f.decl.body, env);
        self.depth -= 1;
        let this = || f.closure.borrow().values.get("this").cloned().unwrap_or(Value::Nil);
        match r {
            Ok(()) if f.is_init => Ok(this()),
            Ok(()) => Ok(Value::Nil),
            Err(Unwind::Return(_)) if f.is_init => Ok(this()),
            Err(Unwind::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }
}

fn binary(op: &Token, l: Value, r: Value) -> Exec<Value> {
    use Value::*;
    Ok(match (op.kind, &l, &r) {
        (T::EqualEqual, ..) => Bool(l.equals(&r)),
        (T::BangEqual, ..) => Bool(!l.equals(&r)),
        (T::Plus, Number(a), Number(b)) => Number(a + b),
        (T::Plus, Str(a), Str(b)) => Str(format!("{a}{b}").into()),
        (T::Plus, ..) => return rt_error(op, "Operands must be two numbers or two strings."),
        (k, Number(a), Number(b)) => match k {
            T::Minus => Number(a - b),
            T::Star => Number(a * b),
            T::Slash => Number(a / b),
            T::Greater => Bool(a > b),
            T::GreaterEqual => Bool(a >= b),
            T::Less => Bool(a < b),
            T::LessEqual => Bool(a <= b),
            _ => unreachable!(),
        },
        _ => return rt_error(op, "Operands must be numbers."),
    })
}
