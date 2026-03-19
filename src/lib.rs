pub mod ast;
pub mod codegen;
pub mod diagnostic;
pub mod hir;
pub mod lexer;
pub mod ownership;
pub mod parser;
pub mod perceus;
pub mod typer;
pub mod types;

pub use codegen::Compiler;
