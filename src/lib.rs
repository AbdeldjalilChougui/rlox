//! rlox: the Lox language from *Crafting Interpreters*, implemented twice:
//! a tree-walk interpreter ([`tree`]) and a bytecode VM ([`vm`]).

pub mod scanner;
pub mod tree;
pub mod vm;

use std::fmt;

/// Error from compiling or running a Lox program.
#[derive(Debug, Clone, PartialEq)]
pub enum LoxError {
    /// Syntax / resolution errors, one formatted message each.
    Compile(Vec<String>),
    /// A runtime error message followed by `[line N]`.
    Runtime(String),
}

impl LoxError {
    /// Conventional sysexits code: 65 for compile errors, 70 for runtime errors.
    pub fn exit_code(&self) -> u8 {
        match self {
            LoxError::Compile(_) => 65,
            LoxError::Runtime(_) => 70,
        }
    }
}

impl fmt::Display for LoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoxError::Compile(errs) => f.write_str(&errs.join("\n")),
            LoxError::Runtime(e) => f.write_str(e),
        }
    }
}

/// Format a number the way Lox prints it (`1` rather than `1.0`).
pub fn format_number(n: f64) -> String {
    if n.is_infinite() {
        return if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    format!("{n}")
}

pub(crate) fn clock_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Which back end executes the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Tree,
    Vm,
}

/// A persistent session (globals survive between `run` calls, as in the REPL).
pub enum Session {
    Tree(tree::Interpreter),
    Vm(vm::Vm),
}

impl Session {
    pub fn new(backend: Backend, out: Box<dyn std::io::Write>) -> Session {
        match backend {
            Backend::Tree => Session::Tree(tree::Interpreter::new(out)),
            Backend::Vm => Session::Vm(vm::Vm::new(out)),
        }
    }

    pub fn run(&mut self, source: &str) -> Result<(), LoxError> {
        match self {
            Session::Tree(i) => i.run(source),
            Session::Vm(v) => v.run(source),
        }
    }
}
