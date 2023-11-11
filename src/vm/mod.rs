//! Bytecode VM (Crafting Interpreters part III, "clox"): a single-pass
//! compiler to a compact bytecode, run by a stack machine with closures
//! via upvalues, classes, bound methods and `super`.

pub mod compiler;
pub mod machine;
pub mod value;

pub use machine::Vm;
