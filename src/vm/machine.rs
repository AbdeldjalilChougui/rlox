//! Stack-based bytecode interpreter.

use super::compiler;
use super::value::{BoundMethod, Class, Closure, Function, Instance, Native, Op, Upvalue, Value};
use crate::LoxError;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

const FRAMES_MAX: usize = 256;

struct Frame {
    closure: Rc<Closure>,
    ip: usize,
    /// Stack index of slot 0 (the callee / `this`).
    base: usize,
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    globals: HashMap<Rc<str>, Value>,
    open_upvalues: Vec<Rc<RefCell<Upvalue>>>,
    out: Box<dyn Write>,
}

type Run<X> = Result<X, String>;

impl Vm {
    pub fn new(out: Box<dyn Write>) -> Vm {
        let mut vm = Vm { stack: Vec::with_capacity(256), frames: Vec::new(), globals: HashMap::new(), open_upvalues: Vec::new(), out };
        vm.define_native("clock", 0, |_| Value::Number(crate::clock_seconds()));
        vm
    }

    fn define_native(&mut self, name: &'static str, arity: usize, f: fn(&[Value]) -> Value) {
        self.globals.insert(name.into(), Value::Native(Rc::new(Native { name, arity, f })));
    }

    pub fn run(&mut self, source: &str) -> Result<(), LoxError> {
        let function = compiler::compile(source).map_err(LoxError::Compile)?;
        let closure = Rc::new(Closure { function: Rc::new(function), upvalues: Vec::new() });
        self.stack.push(Value::Closure(closure.clone()));
        let result = self.call(closure, 0).and_then(|()| self.execute());
        let _ = self.out.flush();
        result.map_err(|msg| {
            let trace = self.stack_trace();
            self.stack.clear();
            self.frames.clear();
            self.open_upvalues.clear();
            LoxError::Runtime(format!("{msg}\n{trace}"))
        })
    }

    fn stack_trace(&self) -> String {
        let mut lines = Vec::new();
        for f in self.frames.iter().rev() {
            let func = &f.closure.function;
            let line = func.chunk.lines.get(f.ip.saturating_sub(1)).copied().unwrap_or(0);
            match &func.name {
                Some(n) => lines.push(format!("[line {line}] in {n}()")),
                None => lines.push(format!("[line {line}] in script")),
            }
        }
        lines.join("\n")
    }

    // ---- bytecode reading -------------------------------------------------

    fn frame(&self) -> &Frame {
        self.frames.last().unwrap()
    }

    fn read_byte(&mut self) -> u8 {
        let f = self.frames.last_mut().unwrap();
        let b = f.closure.function.chunk.code[f.ip];
        f.ip += 1;
        b
    }

    fn read_u16(&mut self) -> usize {
        let hi = self.read_byte() as usize;
        let lo = self.read_byte() as usize;
        (hi << 8) | lo
    }

    fn read_constant(&mut self) -> Value {
        let i = self.read_byte() as usize;
        self.frame().closure.function.chunk.constants[i].clone()
    }

    fn read_string(&mut self) -> Rc<str> {
        match self.read_constant() {
            Value::Str(s) => s,
            v => unreachable!("expected string constant, got {v}"),
        }
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().expect("stack underflow")
    }

    fn peek(&self, distance: usize) -> &Value {
        &self.stack[self.stack.len() - 1 - distance]
    }

    // ---- calls ------------------------------------------------------------

    fn call(&mut self, closure: Rc<Closure>, argc: usize) -> Run<()> {
        if argc != closure.function.arity {
            return Err(format!("Expected {} arguments but got {argc}.", closure.function.arity));
        }
        if self.frames.len() >= FRAMES_MAX {
            return Err("Stack overflow.".into());
        }
        let base = self.stack.len() - argc - 1;
        self.frames.push(Frame { closure, ip: 0, base });
        Ok(())
    }

    fn call_value(&mut self, callee: Value, argc: usize) -> Run<()> {
        match callee {
            Value::Closure(c) => self.call(c, argc),
            Value::Native(n) => {
                if argc != n.arity {
                    return Err(format!("Expected {} arguments but got {argc}.", n.arity));
                }
                let start = self.stack.len() - argc;
                let result = (n.f)(&self.stack[start..]);
                self.stack.truncate(start - 1);
                self.stack.push(result);
                Ok(())
            }
            Value::Class(class) => {
                let slot = self.stack.len() - argc - 1;
                let init = class.borrow().methods.get("init").cloned();
                self.stack[slot] = Value::Instance(Rc::new(RefCell::new(Instance { class, fields: HashMap::new() })));
                match init {
                    Some(init) => self.call(init, argc),
                    None if argc != 0 => Err(format!("Expected 0 arguments but got {argc}.")),
                    None => Ok(()),
                }
            }
            Value::BoundMethod(b) => {
                let slot = self.stack.len() - argc - 1;
                self.stack[slot] = b.receiver.clone();
                self.call(b.method.clone(), argc)
            }
            _ => Err("Can only call functions and classes.".into()),
        }
    }

    fn invoke_from_class(&mut self, class: &Rc<RefCell<Class>>, name: &str, argc: usize) -> Run<()> {
        let method = class.borrow().methods.get(name).cloned();
        match method {
            Some(m) => self.call(m, argc),
            None => Err(format!("Undefined property '{name}'.")),
        }
    }

    fn invoke(&mut self, name: &str, argc: usize) -> Run<()> {
        let Value::Instance(inst) = self.peek(argc).clone() else {
            return Err("Only instances have methods.".into());
        };
        let field = inst.borrow().fields.get(name).cloned();
        if let Some(f) = field {
            let slot = self.stack.len() - argc - 1;
            self.stack[slot] = f.clone();
            return self.call_value(f, argc);
        }
        let class = inst.borrow().class.clone();
        self.invoke_from_class(&class, name, argc)
    }

    fn bind_method(&mut self, class: &Rc<RefCell<Class>>, name: &str) -> Run<()> {
        let method = class.borrow().methods.get(name).cloned();
        let Some(method) = method else {
            return Err(format!("Undefined property '{name}'."));
        };
        let receiver = self.pop();
        self.stack.push(Value::BoundMethod(Rc::new(BoundMethod { receiver, method })));
        Ok(())
    }

    // ---- upvalues ---------------------------------------------------------

    fn capture_upvalue(&mut self, slot: usize) -> Rc<RefCell<Upvalue>> {
        for uv in &self.open_upvalues {
            if matches!(*uv.borrow(), Upvalue::Open(s) if s == slot) {
                return uv.clone();
            }
        }
        let uv = Rc::new(RefCell::new(Upvalue::Open(slot)));
        self.open_upvalues.push(uv.clone());
        uv
    }

    fn close_upvalues(&mut self, from: usize) {
        let stack = &self.stack;
        self.open_upvalues.retain(|uv| {
            let mut u = uv.borrow_mut();
            match *u {
                Upvalue::Open(slot) if slot >= from => {
                    *u = Upvalue::Closed(stack[slot].clone());
                    false
                }
                _ => true,
            }
        });
    }

    fn upvalue_get(&self, uv: &Rc<RefCell<Upvalue>>) -> Value {
        match &*uv.borrow() {
            Upvalue::Open(slot) => self.stack[*slot].clone(),
            Upvalue::Closed(v) => v.clone(),
        }
    }

    // ---- main loop --------------------------------------------------------

    fn number_operands(&mut self) -> Run<(f64, f64)> {
        match (self.peek(1), self.peek(0)) {
            (Value::Number(a), Value::Number(b)) => {
                let r = (*a, *b);
                self.stack.truncate(self.stack.len() - 2);
                Ok(r)
            }
            _ => Err("Operands must be numbers.".into()),
        }
    }

    fn execute(&mut self) -> Run<()> {
        loop {
            let op = Op::from_byte(self.read_byte());
            match op {
                Op::Constant => {
                    let c = self.read_constant();
                    self.stack.push(c);
                }
                Op::Nil => self.stack.push(Value::Nil),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Pop => {
                    self.pop();
                }
                Op::GetLocal => {
                    let slot = self.read_byte() as usize + self.frame().base;
                    self.stack.push(self.stack[slot].clone());
                }
                Op::SetLocal => {
                    let slot = self.read_byte() as usize + self.frame().base;
                    self.stack[slot] = self.peek(0).clone();
                }
                Op::GetGlobal => {
                    let name = self.read_string();
                    match self.globals.get(&name) {
                        Some(v) => self.stack.push(v.clone()),
                        None => return Err(format!("Undefined variable '{name}'.")),
                    }
                }
                Op::DefineGlobal => {
                    let name = self.read_string();
                    let v = self.pop();
                    self.globals.insert(name, v);
                }
                Op::SetGlobal => {
                    let name = self.read_string();
                    let v = self.peek(0).clone();
                    match self.globals.get_mut(&name) {
                        Some(slot) => *slot = v,
                        None => return Err(format!("Undefined variable '{name}'.")),
                    }
                }
                Op::GetUpvalue => {
                    let i = self.read_byte() as usize;
                    let uv = self.frame().closure.upvalues[i].clone();
                    let v = self.upvalue_get(&uv);
                    self.stack.push(v);
                }
                Op::SetUpvalue => {
                    let i = self.read_byte() as usize;
                    let uv = self.frame().closure.upvalues[i].clone();
                    let v = self.peek(0).clone();
                    let slot = match &mut *uv.borrow_mut() {
                        Upvalue::Open(slot) => Some(*slot),
                        Upvalue::Closed(c) => {
                            *c = v.clone();
                            None
                        }
                    };
                    if let Some(slot) = slot {
                        self.stack[slot] = v;
                    }
                }
                Op::GetProperty => {
                    let Value::Instance(inst) = self.peek(0).clone() else {
                        return Err("Only instances have properties.".into());
                    };
                    let name = self.read_string();
                    let field = inst.borrow().fields.get(&name).cloned();
                    match field {
                        Some(v) => {
                            self.pop();
                            self.stack.push(v);
                        }
                        None => {
                            let class = inst.borrow().class.clone();
                            self.bind_method(&class, &name)?;
                        }
                    }
                }
                Op::SetProperty => {
                    let Value::Instance(inst) = self.peek(1).clone() else {
                        return Err("Only instances have fields.".into());
                    };
                    let name = self.read_string();
                    let v = self.pop();
                    inst.borrow_mut().fields.insert(name, v.clone());
                    self.pop();
                    self.stack.push(v);
                }
                Op::GetSuper => {
                    let name = self.read_string();
                    let Value::Class(sc) = self.pop() else { unreachable!("super is a class") };
                    self.bind_method(&sc, &name)?;
                }
                Op::Equal => {
                    let b = self.pop();
                    let a = self.pop();
                    self.stack.push(Value::Bool(a.equals(&b)));
                }
                Op::Greater => {
                    let (a, b) = self.number_operands()?;
                    self.stack.push(Value::Bool(a > b));
                }
                Op::Less => {
                    let (a, b) = self.number_operands()?;
                    self.stack.push(Value::Bool(a < b));
                }
                Op::Add => match (self.peek(1), self.peek(0)) {
                    (Value::Number(a), Value::Number(b)) => {
                        let r = a + b;
                        self.stack.truncate(self.stack.len() - 2);
                        self.stack.push(Value::Number(r));
                    }
                    (Value::Str(a), Value::Str(b)) => {
                        let r: Rc<str> = format!("{a}{b}").into();
                        self.stack.truncate(self.stack.len() - 2);
                        self.stack.push(Value::Str(r));
                    }
                    _ => return Err("Operands must be two numbers or two strings.".into()),
                },
                Op::Subtract => {
                    let (a, b) = self.number_operands()?;
                    self.stack.push(Value::Number(a - b));
                }
                Op::Multiply => {
                    let (a, b) = self.number_operands()?;
                    self.stack.push(Value::Number(a * b));
                }
                Op::Divide => {
                    let (a, b) = self.number_operands()?;
                    self.stack.push(Value::Number(a / b));
                }
                Op::Not => {
                    let v = self.pop();
                    self.stack.push(Value::Bool(v.is_falsey()));
                }
                Op::Negate => match self.pop() {
                    Value::Number(n) => self.stack.push(Value::Number(-n)),
                    _ => return Err("Operand must be a number.".into()),
                },
                Op::Print => {
                    let v = self.pop();
                    let _ = writeln!(self.out, "{v}");
                }
                Op::Jump => {
                    let off = self.read_u16();
                    self.frames.last_mut().unwrap().ip += off;
                }
                Op::JumpIfFalse => {
                    let off = self.read_u16();
                    if self.peek(0).is_falsey() {
                        self.frames.last_mut().unwrap().ip += off;
                    }
                }
                Op::Loop => {
                    let off = self.read_u16();
                    self.frames.last_mut().unwrap().ip -= off;
                }
                Op::Call => {
                    let argc = self.read_byte() as usize;
                    let callee = self.peek(argc).clone();
                    self.call_value(callee, argc)?;
                }
                Op::Invoke => {
                    let name = self.read_string();
                    let argc = self.read_byte() as usize;
                    self.invoke(&name, argc)?;
                }
                Op::SuperInvoke => {
                    let name = self.read_string();
                    let argc = self.read_byte() as usize;
                    let Value::Class(sc) = self.pop() else { unreachable!("super is a class") };
                    self.invoke_from_class(&sc, &name, argc)?;
                }
                Op::Closure => {
                    let Value::Function(function) = self.read_constant() else { unreachable!() };
                    let mut upvalues = Vec::with_capacity(function.upvalue_count);
                    for _ in 0..function.upvalue_count {
                        let is_local = self.read_byte() == 1;
                        let index = self.read_byte() as usize;
                        if is_local {
                            let slot = self.frame().base + index;
                            upvalues.push(self.capture_upvalue(slot));
                        } else {
                            upvalues.push(self.frame().closure.upvalues[index].clone());
                        }
                    }
                    self.stack.push(Value::Closure(Rc::new(Closure { function, upvalues })));
                }
                Op::CloseUpvalue => {
                    self.close_upvalues(self.stack.len() - 1);
                    self.pop();
                }
                Op::Return => {
                    let result = self.pop();
                    let frame = self.frames.pop().unwrap();
                    self.close_upvalues(frame.base);
                    self.stack.truncate(frame.base);
                    if self.frames.is_empty() {
                        return Ok(());
                    }
                    self.stack.push(result);
                }
                Op::Class => {
                    let name = self.read_string();
                    self.stack.push(Value::Class(Rc::new(RefCell::new(Class { name, methods: HashMap::new() }))));
                }
                Op::Inherit => {
                    let Value::Class(sup) = self.peek(1).clone() else {
                        return Err("Superclass must be a class.".into());
                    };
                    let Value::Class(sub) = self.peek(0).clone() else { unreachable!() };
                    let methods = sup.borrow().methods.clone();
                    sub.borrow_mut().methods.extend(methods);
                    self.pop();
                }
                Op::Method => {
                    let name = self.read_string();
                    let Value::Closure(method) = self.pop() else { unreachable!() };
                    let Value::Class(class) = self.peek(0) else { unreachable!() };
                    class.borrow_mut().methods.insert(name, method);
                }
            }
        }
    }
}

/// Expose compiled functions for tests/inspection.
pub fn compile(source: &str) -> Result<Function, Vec<String>> {
    compiler::compile(source)
}
