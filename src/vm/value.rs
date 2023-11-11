//! Runtime values and heap objects for the bytecode VM.
//!
//! Objects are reference counted (`Rc`) instead of traced by a GC, so cycles
//! (e.g. an instance storing a bound method of itself) leak. Fine for a
//! learning VM; see README.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

macro_rules! opcodes {
    ($($name:ident),* $(,)?) => {
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Op { $($name),* }
        impl Op {
            const ALL: &'static [Op] = &[$(Op::$name),*];
            pub fn from_byte(b: u8) -> Op { Op::ALL[b as usize] }
        }
    };
}

opcodes!(
    Constant, Nil, True, False, Pop, GetLocal, SetLocal, GetGlobal, DefineGlobal, SetGlobal,
    GetUpvalue, SetUpvalue, GetProperty, SetProperty, GetSuper, Equal, Greater, Less, Add,
    Subtract, Multiply, Divide, Not, Negate, Print, Jump, JumpIfFalse, Loop, Call, Invoke,
    SuperInvoke, Closure, CloseUpvalue, Return, Class, Inherit, Method,
);

#[derive(Default, Debug)]
pub struct Chunk {
    pub code: Vec<u8>,
    pub lines: Vec<usize>,
    pub constants: Vec<Value>,
}

impl Chunk {
    pub fn write(&mut self, byte: u8, line: usize) {
        self.code.push(byte);
        self.lines.push(line);
    }
}

#[derive(Debug, Default)]
pub struct Function {
    pub name: Option<Rc<str>>,
    pub arity: usize,
    pub upvalue_count: usize,
    pub chunk: Chunk,
}

#[derive(Debug)]
pub enum Upvalue {
    /// Still lives on the VM stack at this absolute slot.
    Open(usize),
    /// Hoisted off the stack when its scope ended.
    Closed(Value),
}

#[derive(Debug)]
pub struct Closure {
    pub function: Rc<Function>,
    pub upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

pub struct Native {
    pub name: &'static str,
    pub arity: usize,
    pub f: fn(&[Value]) -> Value,
}

impl fmt::Debug for Native {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<native {}>", self.name)
    }
}

#[derive(Debug)]
pub struct Class {
    pub name: Rc<str>,
    pub methods: HashMap<Rc<str>, Rc<Closure>>,
}

#[derive(Debug)]
pub struct Instance {
    pub class: Rc<RefCell<Class>>,
    pub fields: HashMap<Rc<str>, Value>,
}

#[derive(Debug)]
pub struct BoundMethod {
    pub receiver: Value,
    pub method: Rc<Closure>,
}

#[derive(Debug, Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    Function(Rc<Function>),
    Closure(Rc<Closure>),
    Native(Rc<Native>),
    Class(Rc<RefCell<Class>>),
    Instance(Rc<RefCell<Instance>>),
    BoundMethod(Rc<BoundMethod>),
}

impl Value {
    pub fn is_falsey(&self) -> bool {
        matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn equals(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Nil, Nil) => true,
            (Bool(a), Bool(b)) => a == b,
            (Number(a), Number(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Function(a), Function(b)) => Rc::ptr_eq(a, b),
            (Closure(a), Closure(b)) => Rc::ptr_eq(a, b),
            (Native(a), Native(b)) => Rc::ptr_eq(a, b),
            (Class(a), Class(b)) => Rc::ptr_eq(a, b),
            (Instance(a), Instance(b)) => Rc::ptr_eq(a, b),
            (BoundMethod(a), BoundMethod(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

fn fn_name(f: &Function) -> String {
    match &f.name {
        Some(n) => format!("<fn {n}>"),
        None => "<script>".into(),
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => f.write_str("nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(n) => f.write_str(&crate::format_number(*n)),
            Value::Str(s) => f.write_str(s),
            Value::Function(func) => f.write_str(&fn_name(func)),
            Value::Closure(c) => f.write_str(&fn_name(&c.function)),
            Value::Native(n) => write!(f, "<native fn {}>", n.name),
            Value::Class(c) => f.write_str(&c.borrow().name),
            Value::Instance(i) => write!(f, "{} instance", i.borrow().class.borrow().name),
            Value::BoundMethod(b) => f.write_str(&fn_name(&b.method.function)),
        }
    }
}
