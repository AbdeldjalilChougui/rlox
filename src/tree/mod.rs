//! Tree-walk interpreter (Crafting Interpreters part II, "jlox").

pub mod ast;
pub mod interpreter;
pub mod parser;
pub mod resolver;

pub use interpreter::Interpreter;
